use std::{
    collections::{hash_map::Entry, BTreeSet, HashMap, VecDeque},
    io,
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
};

use bytes::Bytes;
use ractor::{cast, Actor, ActorRef, RpcReplyPort};
use rosrust::Message;
use tokio::sync::broadcast::{self, error::RecvError};
use tracing::{debug, error, instrument, trace, warn};
use url::Url;

use crate::{
    tcpros::{
        subscription::{Subscription, SubscriptionError},
        CompatibilityError, Topic,
    },
    xmlrpc::{MasterClientError, RosMasterClient, RosSlaveClient, SlaveClientError},
};

#[derive(Debug, thiserror::Error)]
pub enum SubscriberActorError {
    #[error("Master client error: {0}")]
    MasterClient(#[from] MasterClientError),
    #[error("Slave client error: {0}")]
    SlaveClient(#[from] SlaveClientError),
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    #[error("Incompatible messages: {0}")]
    Compatibility(#[from] CompatibilityError),
    #[error(transparent)]
    Subscription(#[from] SubscriptionError),
}

pub type SubscriberActorResult<T> = Result<T, SubscriberActorError>;

pub enum SubscriberActorMsg {
    GetSubscriptions {
        reply: RpcReplyPort<Vec<(String, String)>>,
    },
    GetConnectedPublisherUrls {
        topic_name: String,
        reply: RpcReplyPort<Option<BTreeSet<String>>>,
    },
    RegisterSubscriber {
        topic: Topic,
        caller_id: String,
        queue_size: usize,
        tcp_nodelay: bool,
        reply: RpcReplyPort<SubscriberActorResult<Subscriber>>,
    },
    UnregisterSubscriber {
        topic_name: String,
        reply: RpcReplyPort<SubscriberActorResult<()>>,
    },
    DropSubscriber {
        topic_name: String,
    },
    UpdateConnectedPublishers {
        topic_name: String,
        publishers: BTreeSet<String>,
    },
}

pub struct SubscriberActorState {
    node_name: String,
    master_client: RosMasterClient,
    subscriptions: HashMap<String, (Weak<SubscriberGuard>, Subscription)>,
}

impl SubscriberActorState {
    pub fn new(node_name: &str, master_client: &RosMasterClient) -> Self {
        Self {
            node_name: node_name.to_string(),
            master_client: master_client.clone(),
            subscriptions: HashMap::new(),
        }
    }
}

pub struct SubscriberActor;

impl Actor for SubscriberActor {
    type Msg = SubscriberActorMsg;
    type State = SubscriberActorState;
    type Arguments = SubscriberActorState;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ractor::ActorProcessingErr> {
        Ok(args)
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        for (topic_name, (guard, subscription)) in std::mem::take(&mut state.subscriptions) {
            if let Err(e) = Self::cleanup_subscription(state, guard, subscription).await {
                warn!("Failed to clean up subscription: [topic: \"{topic_name}\", error: \"{e}\"]");
            }
        }

        trace!("Subscriber actor shutdown complete!");
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        match message {
            SubscriberActorMsg::GetSubscriptions { reply } => {
                reply.send(Self::get_subscriptions(state))?;
            }
            SubscriberActorMsg::GetConnectedPublisherUrls { topic_name, reply } => {
                reply.send(Self::get_connected_publisher_urls(state, topic_name).await)?;
            }
            SubscriberActorMsg::RegisterSubscriber {
                topic,
                caller_id,
                queue_size,
                tcp_nodelay,
                reply,
            } => {
                reply.send(
                    Self::register_subscriber(
                        state,
                        myself,
                        topic,
                        caller_id,
                        queue_size,
                        tcp_nodelay,
                    )
                    .await,
                )?;
            }
            SubscriberActorMsg::UnregisterSubscriber { topic_name, reply } => {
                reply.send(Self::unregister_subscriber(state, topic_name).await)?;
            }
            SubscriberActorMsg::DropSubscriber { topic_name } => {
                if let Err(e) = Self::unregister_subscriber(state, topic_name).await {
                    warn!("Encountered an error while handling drop guard message: {e}");
                }
            }
            SubscriberActorMsg::UpdateConnectedPublishers {
                topic_name,
                publishers,
            } => Self::update_connected_publishers(state, topic_name, publishers).await,
        }
        Ok(())
    }
}

impl SubscriberActor {
    async fn cleanup_subscription(
        state: &mut SubscriberActorState,
        guard: Weak<SubscriberGuard>,
        subscription: Subscription,
    ) -> SubscriberActorResult<()> {
        let topic_name = subscription.topic().name.clone();

        trace!("Cleaning up subscription for topic \"{topic_name}\"");

        if let Some(guard) = guard.upgrade() {
            guard.disarm();
        }

        drop(subscription);

        state
            .master_client
            .unregister_subscriber(&topic_name)
            .await?;

        Ok(())
    }

    #[instrument(skip_all)]
    pub fn get_subscriptions(state: &mut SubscriberActorState) -> Vec<(String, String)> {
        trace!("GetSubscriptions called");

        state
            .subscriptions
            .iter()
            .map(|(topic_name, (_, subscription))| {
                (
                    topic_name.clone(),
                    subscription.topic().spec.msg_type.clone(),
                )
            })
            .collect()
    }

    #[instrument(skip(state))]
    pub async fn get_connected_publisher_urls(
        state: &mut SubscriberActorState,
        topic_name: String,
    ) -> Option<BTreeSet<String>> {
        trace!("GetConnectedPublisherUrls called");

        let (_, subscription) = state.subscriptions.get(&topic_name)?;
        Some(subscription.publisher_addrs().await)
    }

    async fn add_publisher_to_subscription(
        caller_id: &str,
        topic_name: &str,
        publisher_addr: &str,
        subscription: &mut Subscription,
    ) -> Result<(), SubscriberActorError> {
        if let Some(channel_addr) =
            Self::get_publisher_channel(caller_id, topic_name, publisher_addr).await?
        {
            debug!("Adding publisher to subscription: [channel_url: \"{channel_addr}\"]");

            subscription
                .add_publisher(publisher_addr, &channel_addr)
                .await?;
        } else {
            warn!("Publisher \"{publisher_addr}\" does not support TCPROS, skipping.")
        }

        Ok(())
    }

    #[instrument(skip_all, fields(caller_id = caller_id, topic_name = topic.name))]
    pub async fn register_subscriber(
        state: &mut SubscriberActorState,
        actor_ref: ActorRef<SubscriberActorMsg>,
        topic: Topic,
        caller_id: String,
        queue_size: usize,
        tcp_nodelay: bool,
    ) -> SubscriberActorResult<Subscriber> {
        trace!("RegisterSubscriber called");

        if let Entry::Occupied(entry) = state.subscriptions.entry(topic.name.clone()) {
            let (guard, subscription) = entry.get();

            // If a subscription already exists, return a new handle pointing to it.
            if let Some(guard) = guard.upgrade() {
                topic
                    .spec
                    .validate_compatibility(&subscription.topic().spec)?;

                /*
                    Publishers who are "latching" will only send messages when the initial
                    TCPROS handshake completes, or when they have new data to send.

                    Since we allow multiple Subscriber handles to share a single Subscription,
                    additional Subscribers created after the initial handshake will miss any latched
                    messages that were sent. To avoid this, the Subscription tracks latched messages
                    so that we can prepopulate Subscriber handles with them.
                */

                let latched_msgs = subscription.latched_msgs().await;

                return Ok(Subscriber::with_latched(
                    latched_msgs.into_iter().collect(),
                    subscription.data_receiver(),
                    guard,
                ));
            }

            warn!("Stale subscription found in registry");

            let (guard, subscription) = entry.remove();

            if let Err(e) = Self::cleanup_subscription(state, guard, subscription).await {
                warn!("Failed to clean up stale subscription: {e}");
            }
        }

        let (data_rx, mut subscription) =
            Subscription::new(&topic, &caller_id, queue_size, tcp_nodelay)?;

        let existing_publishers = state
            .master_client
            .register_subscriber(&topic.name, &topic.spec.msg_type)
            .await?;

        debug!("Found {} existing publisher(s)", existing_publishers.len(),);

        for publisher in existing_publishers {
            if let Err(e) = Self::add_publisher_to_subscription(
                &state.node_name,
                &topic.name,
                &publisher,
                &mut subscription,
            )
            .await
            {
                error!("Failed to add publisher to subscription: {e}");
            }
        }

        let guard = Arc::new(SubscriberGuard::new(&topic.name, &actor_ref));

        state
            .subscriptions
            .insert(topic.name, (Arc::downgrade(&guard), subscription));

        Ok(Subscriber::new(data_rx, guard))
    }

    #[instrument(skip(state))]
    pub async fn unregister_subscriber(
        state: &mut SubscriberActorState,
        topic_name: String,
    ) -> SubscriberActorResult<()> {
        trace!("UnregisterSubscriber called");

        if let Some((guard, subscription)) = state.subscriptions.remove(&topic_name) {
            Self::cleanup_subscription(state, guard, subscription).await?;
        }

        Ok(())
    }

    #[instrument(skip(state, publisher_addrs))]
    pub async fn update_connected_publishers(
        state: &mut SubscriberActorState,
        topic_name: String,
        publisher_addrs: BTreeSet<String>,
    ) {
        trace!("UpdateConnectedPublishers called");

        if let Some((_, subscription)) = state.subscriptions.get_mut(&topic_name) {
            for publisher_addr in publisher_addrs.difference(&subscription.publisher_addrs().await)
            {
                if let Err(e) = Self::add_publisher_to_subscription(
                    &state.node_name,
                    &topic_name,
                    publisher_addr,
                    subscription,
                )
                .await
                {
                    error!("Failed to add publisher to subscription: {e}");
                }
            }
        }
    }

    async fn get_publisher_channel(
        caller_id: &str,
        topic_name: &str,
        publisher_addr: &str,
    ) -> SubscriberActorResult<Option<String>> {
        debug!("Getting topic channel for publisher \"{publisher_addr}\"");

        let publisher_client = RosSlaveClient::new(&Url::parse(publisher_addr)?, caller_id);

        let protocol_info = publisher_client
            .request_topic::<(String, String, i32)>(topic_name, vec![vec!["TCPROS".into()]])
            .await?;

        Ok(protocol_info.map(|info| format!("{}:{}", info.1, info.2)))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SubscriberError {
    #[error("Failed to decode message: {0}")]
    Decode(#[from] io::Error),
    #[error("Failed to receive data from internal channel: {0}")]
    Recv(#[from] RecvError),
}

#[derive(Clone)]
pub struct TypedSubscriber<T> {
    inner: Subscriber,
    _phantom: PhantomData<T>,
}

impl<T> TypedSubscriber<T>
where
    T: Message,
{
    pub fn new(subscriber: Subscriber) -> Self {
        Self {
            inner: subscriber,
            _phantom: PhantomData,
        }
    }

    pub async fn recv(&mut self) -> Result<T, SubscriberError> {
        Ok(T::decode_slice(&self.inner.recv_raw().await?)?)
    }
}

pub struct Subscriber {
    latched_msgs: VecDeque<Bytes>,
    data_rx: broadcast::Receiver<Bytes>,
    _guard: Arc<SubscriberGuard>,
}

impl Subscriber {
    pub fn new(data_rx: broadcast::Receiver<Bytes>, guard: Arc<SubscriberGuard>) -> Self {
        Self {
            data_rx,
            latched_msgs: VecDeque::new(),
            _guard: guard,
        }
    }

    pub fn with_latched(
        latched_msgs: VecDeque<Bytes>,
        data_rx: broadcast::Receiver<Bytes>,
        guard: Arc<SubscriberGuard>,
    ) -> Self {
        Self {
            data_rx,
            latched_msgs,
            _guard: guard,
        }
    }

    pub async fn recv_raw(&mut self) -> Result<Bytes, SubscriberError> {
        Ok(match self.latched_msgs.pop_front() {
            Some(latched_msg) => latched_msg,
            None => self.data_rx.recv().await?,
        })
    }
}

impl Clone for Subscriber {
    fn clone(&self) -> Self {
        Self {
            latched_msgs: self.latched_msgs.clone(),
            data_rx: self.data_rx.resubscribe(),
            _guard: self._guard.clone(),
        }
    }
}

pub struct SubscriberGuard {
    topic_name: String,
    armed: AtomicBool,
    actor_ref: ActorRef<SubscriberActorMsg>,
}

impl SubscriberGuard {
    pub fn new(topic: &str, actor_ref: &ActorRef<SubscriberActorMsg>) -> Self {
        Self {
            topic_name: topic.to_string(),
            armed: AtomicBool::new(true),
            actor_ref: actor_ref.clone(),
        }
    }

    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) {
            if let Err(e) = cast!(
                &self.actor_ref,
                SubscriberActorMsg::DropSubscriber {
                    topic_name: self.topic_name.clone(),
                }
            ) {
                warn!(
                    "Failed to trigger subscription cleanup: [topic: \"{}\", error: \"{e}\"]",
                    self.topic_name
                );
            }
        }
    }
}
