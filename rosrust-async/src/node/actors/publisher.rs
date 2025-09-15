use std::{
    collections::{hash_map::Entry, BTreeSet, HashMap},
    io,
    marker::PhantomData,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
};

use bytes::Bytes;
use ractor::{cast, port::RpcReplyPort, Actor, ActorRef};
use rosrust::Message;
use tokio::sync::broadcast::{self, error::SendError};
use tracing::{debug, instrument, trace, warn};

use crate::{
    tcpros::{
        publication::{Publication, PublicationError},
        CompatibilityError, Topic,
    },
    xmlrpc::{MasterClientError, RosMasterClient},
};

pub enum PublisherActorMsg {
    GetPublications {
        reply: RpcReplyPort<Vec<(String, String)>>,
    },
    GetConnectedSubscriberIDs {
        topic_name: String,
        reply: RpcReplyPort<Option<BTreeSet<String>>>,
    },
    RegisterPublisher {
        topic: Topic,
        address: IpAddr,
        caller_id: String,
        queue_size: usize,
        latching: bool,
        tcp_nodelay: bool,
        reply: RpcReplyPort<PublisherActorResult<Publisher>>,
    },
    UnregisterPublisher {
        topic_name: String,
        reply: RpcReplyPort<PublisherActorResult<()>>,
    },
    DropPublisher {
        topic_name: String,
    },
    RequestTopic {
        topic_name: String,
        reply: RpcReplyPort<Option<SocketAddr>>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PublisherActorError {
    #[error("Master client error: {0}")]
    MasterClient(#[from] MasterClientError),
    #[error("Incompatible messages: {0}")]
    Compatibility(#[from] CompatibilityError),
    #[error(transparent)]
    Publication(#[from] PublicationError),
}

pub type PublisherActorResult<T> = Result<T, PublisherActorError>;

pub struct PublisherActorState {
    master_client: RosMasterClient,
    publications: HashMap<String, (Weak<PublisherGuard>, Publication)>,
}

impl PublisherActorState {
    pub fn new(master_client: &RosMasterClient) -> Self {
        Self {
            master_client: master_client.clone(),
            publications: HashMap::new(),
        }
    }
}

pub struct PublisherActor;

impl Actor for PublisherActor {
    type Msg = PublisherActorMsg;
    type State = PublisherActorState;
    type Arguments = PublisherActorState;

    async fn pre_start(
        &self,
        _myself: ractor::ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ractor::ActorProcessingErr> {
        Ok(args)
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        for (_, (guard, publication)) in std::mem::take(&mut state.publications) {
            if let Err(e) = Self::cleanup_publication(state, guard, publication).await {
                warn!("Failed to clean up publication: {e}");
            }
        }

        trace!("Publisher actor shutdown complete!");
        Ok(())
    }

    async fn handle(
        &self,
        myself: ractor::ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        match message {
            PublisherActorMsg::GetPublications { reply } => {
                reply.send(Self::get_publications(state))?;
            }

            PublisherActorMsg::GetConnectedSubscriberIDs { topic_name, reply } => {
                reply.send(Self::get_connected_subscriber_ids(state, topic_name).await)?;
            }

            PublisherActorMsg::RegisterPublisher {
                topic,
                address,
                caller_id,
                queue_size,
                latching,
                tcp_nodelay,
                reply,
            } => {
                reply.send(
                    Self::register_publisher(
                        state,
                        myself,
                        topic,
                        address,
                        caller_id,
                        queue_size,
                        latching,
                        tcp_nodelay,
                    )
                    .await,
                )?;
            }

            PublisherActorMsg::UnregisterPublisher { topic_name, reply } => {
                reply.send(Self::unregister_publisher(state, topic_name).await)?;
            }
            PublisherActorMsg::DropPublisher { topic_name } => {
                if let Err(e) = Self::unregister_publisher(state, topic_name).await {
                    warn!("Encountered an error while handling drop guard message: {e}");
                }
            }
            PublisherActorMsg::RequestTopic { topic_name, reply } => {
                reply.send(Self::request_topic(state, topic_name).await)?;
            }
        }

        Ok(())
    }
}

impl PublisherActor {
    async fn cleanup_publication(
        state: &PublisherActorState,
        guard: Weak<PublisherGuard>,
        publication: Publication,
    ) -> PublisherActorResult<()> {
        if let Some(guard) = guard.upgrade() {
            guard.disarm();
        }

        state
            .master_client
            .unregister_publisher(&publication.topic().name)
            .await?;
        Ok(())
    }

    pub fn get_publications(state: &mut PublisherActorState) -> Vec<(String, String)> {
        trace!("GetPublications called");

        state
            .publications
            .iter()
            .map(|(topic_name, (_, publication))| {
                (topic_name.clone(), publication.topic().spec.msg_type.clone())
            })
            .collect()
    }

    #[instrument(skip(state))]
    pub async fn get_connected_subscriber_ids(
        state: &mut PublisherActorState,
        topic_name: String,
    ) -> Option<BTreeSet<String>> {
        trace!("GetConnectedSubscriberIDs called");

        let (_, publication) = state.publications.get(&topic_name)?;
        Some(publication.subscriber_ids().await)
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip_all, fields(caller_id = caller_id, topic_name = topic.name))]
    pub async fn register_publisher(
        state: &mut PublisherActorState,
        actor_ref: ActorRef<PublisherActorMsg>,
        topic: Topic,
        address: IpAddr,
        caller_id: String,
        queue_size: usize,
        latching: bool,
        tcp_nodelay: bool,
    ) -> PublisherActorResult<Publisher> {
        trace!("RegisterPublisher called");

        if let Entry::Occupied(entry) = state.publications.entry(topic.name.clone()) {
            let (guard, publication) = entry.get();

            // If a publication already exists, return a new handle pointing to it.
            if let Some(guard) = guard.upgrade() {
                topic.spec.validate_compatibility(&publication.topic().spec)?;

                return Ok(Publisher::new(publication.data_sender(), guard));
            }

            warn!("Stale publication found in registry");

            let (guard, publication) = entry.remove();

            if let Err(e) = Self::cleanup_publication(state, guard, publication).await {
                warn!("Failed to clean up stale publication: {e}");
            }
        }

        let (data_tx, publication) = Publication::new(
            SocketAddr::new(address, 0),
            &topic,
            &caller_id,
            queue_size,
            tcp_nodelay,
            latching,
        )
        .await?;

        let existing_subscribers = state
            .master_client
            .register_publisher(&topic.name, &topic.spec.msg_type)
            .await?;

        debug!(
            "Found {} existing subscriber(s)",
            existing_subscribers.len()
        );

        let guard = Arc::new(PublisherGuard::new(&topic.name, &actor_ref));

        state
            .publications
            .insert(topic.name, (Arc::downgrade(&guard), publication));

        Ok(Publisher::new(data_tx, guard))
    }

    #[instrument(skip(state))]
    pub async fn unregister_publisher(
        state: &mut PublisherActorState,
        topic_name: String,
    ) -> PublisherActorResult<()> {
        trace!("UnregisterPublisher called");

        if let Some((guard, publication)) = state.publications.remove(&topic_name) {
            Self::cleanup_publication(state, guard, publication).await?;
        }

        Ok(())
    }

    #[instrument(skip(state))]
    pub async fn request_topic(
        state: &mut PublisherActorState,
        topic_name: String,
    ) -> Option<SocketAddr> {
        trace!("RequestTopic called");

        state
            .publications
            .get(&topic_name)
            .map(|(_, publication)| publication.address().to_owned())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum PublisherError {
    #[error("Failed to encode message: {0}")]
    Encode(#[from] io::Error),
    #[error("Failed to send message to internal channel: {0}")]
    Send(#[from] SendError<Bytes>),
}

#[derive(Clone)]
pub struct TypedPublisher<T> {
    inner: Publisher,
    _phantom: PhantomData<T>,
}

impl<T> TypedPublisher<T>
where
    T: Message,
{
    pub fn new(publisher: Publisher) -> Self {
        Self {
            inner: publisher,
            _phantom: PhantomData,
        }
    }

    pub fn send(&self, message: &T) -> Result<(), PublisherError> {
        self.inner.send_raw(Bytes::from(message.encode_vec()?))
    }
}

#[derive(Clone)]
pub struct Publisher {
    data_tx: broadcast::Sender<Bytes>,
    _guard: Arc<PublisherGuard>,
}

impl Publisher {
    pub fn new(data_tx: broadcast::Sender<Bytes>, guard: Arc<PublisherGuard>) -> Self {
        Self {
            data_tx,
            _guard: guard,
        }
    }

    pub fn send_raw(&self, data: impl Into<Bytes>) -> Result<(), PublisherError> {
        self.data_tx.send(data.into())?;
        Ok(())
    }
}

pub struct PublisherGuard {
    topic_name: String,
    armed: AtomicBool,
    actor_ref: ActorRef<PublisherActorMsg>,
}

impl PublisherGuard {
    pub fn new(topic: &str, actor_ref: &ActorRef<PublisherActorMsg>) -> Self {
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

impl Drop for PublisherGuard {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) {
            if let Err(e) = cast!(
                &self.actor_ref,
                PublisherActorMsg::DropPublisher {
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
