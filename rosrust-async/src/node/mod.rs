use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    net::SocketAddr,
    sync::Arc,
};

use async_shutdown::{ShutdownComplete, ShutdownManager};
use dxr::{TryFromValue, TryToValue};
use futures_util::FutureExt;
use md5::{Digest, Md5};
use ractor::{call, Actor, ActorRef};
use ros_message::{MessagePath, Msg};
use rosrust::{DynamicMsg, Message, RosMsg, ServicePair};
use tokio::{net::TcpListener, sync::broadcast::error::RecvError, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, span, trace, warn, Instrument, Level};
use url::Url;

mod actors;
mod api;
mod error;

pub mod builder;

use {
    crate::{
        tcpros::{service::server::CallbackError, Service, Topic, TopicSpec},
        xmlrpc::RosMasterClient,
    },
    actors::{
        parameter::{ParameterActor, ParameterActorMsg, ParameterActorState},
        publisher::{PublisherActor, PublisherActorMsg, PublisherActorState},
        service::{ServiceActor, ServiceActorMsg, ServiceActorState, ServiceServer},
        subscriber::{SubscriberActor, SubscriberActorMsg, SubscriberActorState},
    },
    api::{router, server::Server},
};

pub use {
    actors::{
        parameter::ParameterActorError,
        publisher::{Publisher, PublisherActorError, PublisherError, TypedPublisher},
        service::{ServiceActorError, ServiceClient, ServiceClientError, TypedServiceClient},
        subscriber::{Subscriber, SubscriberActorError, SubscriberError, TypedSubscriber},
    },
    error::NodeError,
};

type NodeResult<T> = Result<T, NodeError>;

#[derive(Clone)]
pub struct Node {
    state: Arc<NodeState>,
}

struct NodeState {
    name: String,
    hostname: String,
    address: SocketAddr,
    api_url: Url,
    master_url: Url,
    pub_actor: ActorRef<PublisherActorMsg>,
    sub_actor: ActorRef<SubscriberActorMsg>,
    svc_actor: ActorRef<ServiceActorMsg>,
    param_actor: ActorRef<ParameterActorMsg>,
    shutdown_mgr: ShutdownManager<Option<String>>,
}

impl Node {
    /// Construct a new Node.
    pub async fn new(
        name: impl Into<String>,
        hostname: impl Into<String>,
        api_listener: TcpListener,
        master_url: Url,
    ) -> NodeResult<Self> {
        let name = name.into();
        let hostname = hostname.into();
        let bind_address = api_listener.local_addr()?;
        let api_url = Url::parse(&format!("http://{}:{}", hostname, bind_address.port())).unwrap();

        info!("Launching node: [name: \"{name}\", url: \"{api_url}\", bound_addr: \"{bind_address}\", master_url: \"{master_url}\"]");

        let master_client = RosMasterClient::new(&master_url, &name, api_url.to_string());

        let (pub_actor, _) = Actor::spawn(
            None,
            PublisherActor,
            PublisherActorState::new(&master_client),
        )
        .await?;

        let (sub_actor, _) = Actor::spawn(
            None,
            SubscriberActor,
            SubscriberActorState::new(&name, &master_client),
        )
        .await?;

        let (param_actor, _) = Actor::spawn(
            None,
            ParameterActor,
            ParameterActorState::new(&master_client),
        )
        .await?;

        let (svc_actor, _) =
            Actor::spawn(None, ServiceActor, ServiceActorState::new(&master_client)).await?;

        let actor_cells = vec![
            pub_actor.get_cell(),
            sub_actor.get_cell(),
            svc_actor.get_cell(),
            param_actor.get_cell(),
        ];

        let shutdown_mgr = ShutdownManager::new();

        let state = Arc::new(NodeState {
            name,
            hostname,
            address: bind_address,
            api_url,
            master_url,
            pub_actor,
            sub_actor,
            svc_actor,
            param_actor,
            shutdown_mgr: shutdown_mgr.clone(),
        });

        let (api_server, api_shutdown_trigger) = Server::new(router::build_router(&state));

        tokio::spawn(async move {
            let _shutdown_guard = shutdown_mgr.delay_shutdown_token().ok();

            tokio::select! {
                shutdown_trigger = shutdown_mgr.wait_shutdown_triggered() => {
                    match shutdown_trigger{
                        Some(reason) => info!("Node shutdown requested with reason: \"{reason}\""),
                        None => info!("Node shutdown requested with no reason"),
                    }
                }

                Err(e) = api_server.serve_listener(api_listener) => {
                    error!("Failed to serve XML-RPC API: {e}");
                }
            }

            api_shutdown_trigger.notify_waiters();

            for actor in actor_cells {
                if let Err(e) = actor.stop_and_wait(None, None).await {
                    warn!(
                        "Failed to stop actor: {}: {e}",
                        actor
                            .get_name()
                            .unwrap_or_else(|| actor.get_id().to_string())
                    );
                }
            }
        });

        Ok(Node { state })
    }

    /// Get the node's name.
    pub fn name(&self) -> &str {
        &self.state.name
    }

    /// Get the node's hostname.
    pub fn hostname(&self) -> &str {
        &self.state.hostname
    }

    /// Get the address that the node's XML-RPC server is bound to.
    pub fn address(&self) -> &SocketAddr {
        &self.state.address
    }

    /// Get the node's XML-RPC URL.
    pub fn url(&self) -> &Url {
        &self.state.api_url
    }

    /// Get the master URL that the node is connected to.
    pub fn master_url(&self) -> &Url {
        &self.state.master_url
    }

    /// Check if the node has been shut down.
    pub fn is_shutdown(&self) -> bool {
        self.state.shutdown_mgr.is_shutdown_completed()
    }

    /// Trigger a graceful shutdown without waiting for it to complete.
    pub fn shutdown(&self, reason: Option<String>) -> NodeResult<()> {
        if !self.state.shutdown_mgr.is_shutdown_triggered() {
            self.state.shutdown_mgr.trigger_shutdown(reason)?;
        }

        Ok(())
    }

    /// Trigger a graceful shutdown and wait for it to complete.
    ///
    /// It is highly recommended to call this method when you are done with a Node.
    /// The ROS Master lacks a mechanism to detect and clean up stale resources, so
    /// dropping a Node without calling this method will leave the master in an invalid
    /// state.
    ///
    /// The lack of an async `Drop` makes this challenging. We could call [`shutdown`]
    /// inside the `Drop` impl, however there is no guarantee that resources will be
    /// cleaned up. This is especially true in situations where an application is exiting,
    /// since often times the async runtime will be unavailable.
    ///
    /// [`shutdown`]: method@Self::shutdown
    pub async fn shutdown_and_wait(self, reason: Option<String>) -> NodeResult<()> {
        self.shutdown(reason)?;
        self.state.shutdown_mgr.wait_shutdown_complete().await;
        Ok(())
    }

    /// Returns a future that resolves once the node has completely shut down,
    /// with an optional shutdown reason.
    pub fn shutdown_complete(&self) -> ShutdownComplete<Option<String>> {
        self.state.shutdown_mgr.wait_shutdown_complete()
    }

    /// Create a new ROS publication.
    ///
    /// Returns a handle pointing to the underlying publication.
    ///
    /// If a publication already exists for the provided topic, then a cloned
    /// handle will be returned. The publication will be automatically
    /// cleaned up once all handles are dropped.
    pub async fn publish<T>(
        &self,
        topic_name: impl Into<String>,
        queue_size: usize,
        latching: bool,
        tcp_nodelay: bool,
    ) -> NodeResult<TypedPublisher<T>>
    where
        T: Message,
    {
        let publisher = call!(self.state.pub_actor, |reply| {
            PublisherActorMsg::RegisterPublisher {
                topic: Topic::new::<T>(topic_name),
                address: self.state.address.ip(),
                caller_id: self.state.name.clone(),
                queue_size,
                latching,
                tcp_nodelay,
                reply,
            }
        })??;

        Ok(TypedPublisher::<T>::new(publisher))
    }

    fn hash_message(
        root: &DynamicMsg,
        msg: &Msg,
        hashes: &mut HashMap<MessagePath, String>,
    ) -> Result<String, ros_message::Error> {
        for dep_path in msg.dependencies() {
            if !hashes.contains_key(&dep_path) {
                if let Some(dep_msg) = root.dependency(&dep_path) {
                    Self::hash_message(root, dep_msg, hashes)?;
                }
            }
        }

        let mut hasher = Md5::new();
        hasher.update(msg.get_md5_representation(hashes)?);
        let hash_string = format!("{:x}", hasher.finalize());
        hashes.insert(msg.path().clone(), hash_string.clone());

        Ok(hash_string)
    }

    /// Create a new ROS publication using a message type and definition.
    ///
    /// Useful in situations where you may not know the message type at compile-time.
    /// It is recommended to stick with the [`publish`] method which covers most common
    /// use-cases.
    ///
    /// [`publish`]: method@Self::publish
    pub async fn publish_dynamic(
        &self,
        topic_name: impl Into<String>,
        message_type: impl Into<String>,
        message_definition: impl Into<String>,
        queue_size: usize,
        latching: bool,
        tcp_nodelay: bool,
    ) -> NodeResult<Publisher> {
        let message_type = message_type.into();
        let message_definition = message_definition.into();

        // todo: We should probably roll our own minimal version of DynamicMsg to allow for better error types
        let dyn_message = DynamicMsg::new(&message_type, &message_definition)
            .map_err(|e| NodeError::InvalidDyamicMsg(e.to_string()))?;

        let mut msg_hashes = HashMap::new();

        let md5sum = Self::hash_message(&dyn_message, dyn_message.msg(), &mut msg_hashes)?;

        let topic = Topic {
            name: topic_name.into(),
            spec: TopicSpec {
                md5sum,
                msg_type: message_type,
                msg_definition: message_definition,
            },
        };

        let publisher = call!(self.state.pub_actor, |reply| {
            PublisherActorMsg::RegisterPublisher {
                topic,
                address: self.state.address.ip(),
                caller_id: self.state.name.clone(),
                queue_size,
                latching,
                tcp_nodelay,
                reply,
            }
        })??;

        Ok(publisher)
    }

    /// Get a list of all active publications.
    pub async fn get_publications(&self) -> NodeResult<Vec<(String, String)>> {
        Ok(call!(self.state.pub_actor, |reply| {
            PublisherActorMsg::GetPublications { reply }
        })?)
    }

    /// Get a list of connected subscribers for a given topic.
    ///
    /// Returns a list of ROS caller IDs.
    ///
    /// If no subscribers are connected or the node is not publishing
    /// on the provided topic, then no results will be returned.
    pub async fn get_connected_subscriber_ids(
        &self,
        topic_name: impl Into<String>,
    ) -> NodeResult<Option<BTreeSet<String>>> {
        Ok(call!(self.state.pub_actor, |reply| {
            PublisherActorMsg::GetConnectedSubscriberIDs {
                topic_name: topic_name.into(),
                reply,
            }
        })?)
    }

    /// Create a new ROS subscription.
    ///
    /// Returns a handle pointing to the underlying subscription.
    ///
    /// If a subscription already exists for the provided topic, then a cloned
    /// handle will be returned. The subscription will be automatically
    /// cleaned up once all handles are dropped.
    pub async fn subscribe<T: Message>(
        &self,
        topic_name: impl Into<String>,
        queue_size: usize,
        tcp_nodelay: bool,
    ) -> NodeResult<TypedSubscriber<T>> {
        let subscriber = call!(self.state.sub_actor, |reply| {
            SubscriberActorMsg::RegisterSubscriber {
                topic: Topic::new::<T>(topic_name),
                caller_id: self.state.name.clone(),
                queue_size,
                tcp_nodelay,
                reply,
            }
        })??;

        Ok(TypedSubscriber::new(subscriber))
    }

    /// Create a new ROS subscription that triggers a callback for each message received.
    ///
    /// Returns a CancellationToken and JoinHandle for controlling the underlying task.
    ///
    /// This is offered as a convenience for when you don't need full control of the
    /// subscriber handle. Consider using [`subscribe`] for more complex scenarios.
    ///
    /// [`subscribe`]: method@Self::subscribe
    pub async fn subscribe_callback<T, F, Fut>(
        &self,
        topic_name: impl Into<String>,
        queue_size: usize,
        tcp_nodelay: bool,
        callback: F,
    ) -> NodeResult<(CancellationToken, JoinHandle<()>)>
    where
        T: Message,
        F: Fn(T) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let topic_name = topic_name.into();
        let cancel_token = CancellationToken::new();
        let task_cancel_token = cancel_token.clone();

        let mut subscriber = self
            .subscribe::<T>(topic_name.clone(), queue_size, tcp_nodelay)
            .await?;

        //todo: perhaps a unique span suffix to distinguish multiple callbacks for the same topic?
        let span = span!(
            parent: None,
            Level::DEBUG,
            "subscriber_callback",
            topic = topic_name,
        );

        let task_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = task_cancel_token.cancelled() => {
                        trace!("Subscriber callback stopped by cancel token: [topic: \"{topic_name}\"]");
                        break;
                    }

                    msg = subscriber.recv() => {
                        match msg{
                            Ok(msg) => {
                                callback(msg).await;
                            }
                            Err(SubscriberError::Recv(RecvError::Lagged(lagged))) => {
                                // This might be a problem if used on high-frequency topics, maybe look into rate limiting?
                                warn!("Subscriber callback is lagging, dropped {lagged} message(s)");
                            }
                            Err(SubscriberError::Recv(RecvError::Closed)) => {
                                debug!("Internal data channel for subscriber callback was closed");
                                break;
                            }
                            Err(e) => {
                                warn!("Error encountered in subscriber callback: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        }.instrument(span));

        Ok((cancel_token, task_handle))
    }

    /// Get a list of topics that this node is subscribed to.
    pub async fn get_subscriptions(&self) -> NodeResult<Vec<(String, String)>> {
        Ok(call!(self.state.sub_actor, |reply| {
            SubscriberActorMsg::GetSubscriptions { reply }
        })?)
    }

    /// Get a list of connected publishers for a given topic.
    ///
    /// Returns a list of XML-RCP URIs.
    ///
    /// If no publishers are connected or the node is not subscribed
    /// to the provided topic, then no results will be returned.
    pub async fn get_connected_publisher_urls(
        &self,
        topic_name: impl Into<String>,
    ) -> NodeResult<Option<BTreeSet<String>>> {
        Ok(call!(self.state.sub_actor, |reply| {
            SubscriberActorMsg::GetConnectedPublisherUrls {
                topic_name: topic_name.into(),
                reply,
            }
        })?)
    }

    /// Create a new ROS service client.
    pub async fn service_client<T>(
        &self,
        service_name: impl Into<String>,
        persistent: bool,
    ) -> NodeResult<TypedServiceClient<T>>
    where
        T: ServicePair,
    {
        let client = call!(self.state.svc_actor, |reply| {
            ServiceActorMsg::RegisterClient {
                service: Service::new::<T>(service_name),
                caller_id: self.state.name.clone(),
                persistent,
                reply,
            }
        })??;

        Ok(TypedServiceClient::new(client))
    }

    /// Advertise a ROS service.
    ///
    /// Services should be used for operations that terminate quickly, such as querying the state
    /// of a node or performing quick calculations. They should never be used for long-running processes
    pub async fn advertise_service<T, F, Fut>(
        &self,
        service_name: impl Into<String>,
        callback: F,
    ) -> NodeResult<ServiceServer>
    where
        T: ServicePair,
        F: Fn(T::Request) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T::Response, CallbackError>> + Send + 'static,
    {
        let erased_callback = Arc::new(move |data: Vec<u8>| {
            let callback = callback.clone();
            async move {
                let request = T::Request::decode_slice(&data)?;
                let response = callback(request).await?;
                Ok(response.encode_vec()?)
            }
            .boxed()
        });

        Ok(call!(self.state.svc_actor, |reply| {
            ServiceActorMsg::RegisterService {
                service: Service::new::<T>(service_name),
                address: self.state.address.ip(),
                caller_id: self.state.name.clone(),
                callback: erased_callback,
                reply,
            }
        })??)
    }

    /// Get a list of services that this node is advertising.
    pub async fn get_services(&self) -> NodeResult<Vec<(String, String)>> {
        Ok(call!(self.state.svc_actor, |reply| {
            ServiceActorMsg::GetServices { reply }
        })?)
    }

    /// Get a parameter from the ROS parameter server.
    pub async fn get_param<T: TryFromValue>(
        &self,
        name: impl Into<String>,
    ) -> NodeResult<Option<T>> {
        let raw_param = call!(self.state.param_actor, |reply| {
            ParameterActorMsg::GetParam {
                name: name.into(),
                reply,
            }
        })??;

        Ok(raw_param
            .map(|param| T::try_from_value(&param))
            .transpose()?)
    }

    /// Get a parameter from the ROS parameter server, with local caching.
    ///
    /// This method also subscribes the node to any future updates to this parameter,
    /// ensuring the cached value matches what is on the parameter server.
    pub async fn get_param_cached<T: TryFromValue>(
        &self,
        name: impl Into<String>,
    ) -> NodeResult<Option<T>> {
        let raw_param = call!(self.state.param_actor, |reply| {
            ParameterActorMsg::GetCachedParam {
                name: name.into(),
                reply,
            }
        })??;

        Ok(raw_param
            .map(|param| T::try_from_value(&param))
            .transpose()?)
    }

    /// Store a parameter in the ROS parameter server.
    pub async fn set_param<T: TryToValue>(
        &self,
        name: impl Into<String>,
        value: T,
    ) -> NodeResult<()> {
        let value = value.try_to_value()?;

        Ok(call!(self.state.param_actor, |reply| {
            ParameterActorMsg::SetParam {
                name: name.into(),
                value,
                reply,
            }
        })??)
    }

    /// Delete a parameter from the ROS parameter server.
    pub async fn delete_param(&self, name: impl Into<String>) -> NodeResult<()> {
        Ok(call!(self.state.param_actor, |reply| {
            ParameterActorMsg::DeleteParam {
                name: name.into(),
                reply,
            }
        })??)
    }

    /// Check if a parameter exists in the ROS parameter server.
    pub async fn has_param(&self, name: impl Into<String>) -> NodeResult<bool> {
        Ok(call!(self.state.param_actor, |reply| {
            ParameterActorMsg::ParamExists {
                name: name.into(),
                reply,
            }
        })??)
    }

    /// Search for a parameter key in the ROS parameter server.
    ///
    /// The search starts in caller's namespace and proceeds upwards through
    /// parent namespaces until the server finds a matching key.
    pub async fn search_param(&self, name: impl Into<String>) -> NodeResult<Option<String>> {
        Ok(call!(self.state.param_actor, |reply| {
            ParameterActorMsg::SearchParam {
                name: name.into(),
                reply,
            }
        })??)
    }

    /// Get a list of all parameters in the ROS parameter server.
    pub async fn get_param_names(&self) -> NodeResult<Vec<String>> {
        Ok(call!(self.state.param_actor, |reply| {
            ParameterActorMsg::GetParamNames { reply }
        })??)
    }
}
