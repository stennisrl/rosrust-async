use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    marker::PhantomData,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
};

use ractor::{cast, Actor, ActorRef, RpcReplyPort};
use rosrust::RosMsg;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, instrument, trace, warn};

use crate::{
    tcpros::{
        header::HeaderError,
        service::{
            client::{ClientLinkError, RpcMsg, ServiceClientLink},
            server::{ErasedCallback, ServiceProvider, ServiceProviderError},
        },
        CompatibilityError, Service,
    },
    xmlrpc::{MasterClientError, RosMasterClient},
};

#[derive(thiserror::Error, Debug)]
pub enum ServiceActorError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Master client error: {0}")]
    MasterClient(#[from] MasterClientError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("Incompatible messages: {0}")]
    Compatibility(#[from] CompatibilityError),
    #[error(transparent)]
    ServiceClientLink(#[from] ClientLinkError),
    #[error(transparent)]
    ServiceProvider(#[from] ServiceProviderError),
}

pub type ServiceActorResult<D> = Result<D, ServiceActorError>;

pub enum ServiceActorMsg {
    RegisterClient {
        service: Service,
        caller_id: String,
        persistent: bool,
        reply: RpcReplyPort<ServiceActorResult<ServiceClient>>,
    },
    DropClient {
        service_name: String,
    },
    GetServices {
        reply: RpcReplyPort<Vec<(String, String)>>,
    },
    RegisterService {
        service: Service,
        address: IpAddr,
        caller_id: String,
        callback: ErasedCallback,
        reply: RpcReplyPort<ServiceActorResult<ServiceServer>>,
    },
    DropService {
        service_name: String,
    },
}

pub struct ServiceActorState {
    master_client: RosMasterClient,
    clients: HashMap<String, (Weak<ServiceClientGuard>, ServiceClientLink)>,
    servers: HashMap<String, (Weak<ServiceServerGuard>, ServiceProvider)>,
}

impl ServiceActorState {
    pub fn new(master_client: &RosMasterClient) -> Self {
        Self {
            master_client: master_client.clone(),
            clients: HashMap::new(),
            servers: HashMap::new(),
        }
    }
}

pub struct ServiceActor;

impl Actor for ServiceActor {
    type Msg = ServiceActorMsg;
    type State = ServiceActorState;
    type Arguments = ServiceActorState;

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
        for (service_name, (guard, server)) in std::mem::take(&mut state.servers) {
            trace!("Cleaning up server for service \"{service_name}\"");
            Self::cleanup_server(state, guard, server).await?;
        }

        for (service_name, (guard, _client)) in std::mem::take(&mut state.clients) {
            trace!("Cleaning up client for service \"{service_name}\"");

            if let Some(guard) = guard.upgrade() {
                guard.disarm();
            }
        }

        trace!("Service actor shutdown complete!");
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        match message {
            ServiceActorMsg::RegisterClient {
                service,
                caller_id,
                persistent,
                reply,
            } => reply.send(
                Self::register_client(myself, state, &service, &caller_id, persistent).await,
            )?,
            ServiceActorMsg::DropClient { service_name } => {
                Self::unregister_client(state, service_name)
            }
            ServiceActorMsg::GetServices { reply } => reply.send(Self::get_services(state))?,
            ServiceActorMsg::RegisterService {
                service,
                address,
                caller_id,
                callback,
                reply,
            } => reply.send(
                Self::register_service(state, myself, service, address, caller_id, callback).await,
            )?,
            ServiceActorMsg::DropService { service_name } => {
                if let Err(e) = Self::unregister_service(state, service_name).await {
                    warn!("Encountered an error while handling drop guard message: {e}");
                }
            }
        }

        Ok(())
    }
}

impl ServiceActor {
    async fn cleanup_server(
        state: &mut ServiceActorState,
        guard: Weak<ServiceServerGuard>,
        server: ServiceProvider,
    ) -> ServiceActorResult<()> {
        let service_name = server.service().name.clone();
        trace!("Cleaning up server for service \"{service_name}\"");

        if let Some(guard) = guard.upgrade() {
            guard.disarm();
        }

        state
            .master_client
            .unregister_service(&service_name, server.url().as_str())
            .await?;

        Ok(())
    }

    #[instrument(skip_all, fields(caller_id = caller_id, service_name = service.name))]
    pub async fn register_client(
        actor_ref: ActorRef<ServiceActorMsg>,
        state: &mut ServiceActorState,
        service: &Service,
        caller_id: &str,
        persistent: bool,
    ) -> ServiceActorResult<ServiceClient> {
        trace!("RegisterClient called");

        if let Entry::Occupied(entry) = state.clients.entry(service.name.clone()) {
            let (guard, client) = entry.get();

            // If a client link already exists, return a new handle pointing to it.
            if let Some(guard) = guard.upgrade() {
                service.spec.validate_compatibility(&client.service().spec)?;

                return Ok(ServiceClient::new(client.rpc_sender(), guard));
            }

            warn!("Stale service client found in registry",);

            entry.remove();
        }

        // This is the only instance where we mix XML-RPC with the low-level TCPROS code.
        // Non-persistent ROS1 service clients need to use the lookupService method before
        // each call, which requires the ServiceClientLink to have a master client of its own.
        let (call_tx, client) =
            ServiceClientLink::new(service, caller_id, persistent, state.master_client.clone())
                .await?;

        let guard = Arc::new(ServiceClientGuard::new(&service.name, &actor_ref));

        state
            .clients
            .insert(service.name.clone(), (Arc::downgrade(&guard), client));

        Ok(ServiceClient::new(call_tx, guard))
    }

    #[instrument(skip(state))]
    pub fn unregister_client(state: &mut ServiceActorState, service_name: String) {
        trace!("UnregisterClient called");
        state.clients.remove(&service_name);
    }

    #[instrument(skip_all)]
    pub fn get_services(state: &mut ServiceActorState) -> Vec<(String, String)> {
        trace!("GetServices called");

        state
            .servers
            .iter()
            .map(|(service_name, (_, server))| {
                (service_name.clone(), server.service().spec.msg_type.clone())
            })
            .collect()
    }

    #[instrument(skip_all, fields(caller_id = caller_id, service_name = service.name))]
    pub async fn register_service(
        state: &mut ServiceActorState,
        actor_ref: ActorRef<ServiceActorMsg>,
        service: Service,
        address: IpAddr,
        caller_id: String,
        callback: ErasedCallback,
    ) -> ServiceActorResult<ServiceServer> {
        trace!("RegisterService called");

        if let Entry::Occupied(entry) = state.servers.entry(service.name.clone()) {
            let (guard, server) = entry.get();

            // If a client link already exists, return a new handle pointing to it.
            if let Some(guard) = guard.upgrade() {
                service.spec.validate_compatibility(&server.service().spec)?;

                return Ok(ServiceServer::new(guard));
            }

            warn!("Stale service server found in registry",);

            entry.remove();
        }

        let srv = ServiceProvider::new(SocketAddr::new(address, 0), &service, &caller_id, callback)
            .await?;

        let guard = Arc::new(ServiceServerGuard::new(&service.name, &actor_ref));

        state
            .master_client
            .register_service(&service.name, srv.url().as_str())
            .await?;

        state
            .servers
            .insert(service.name, (Arc::downgrade(&guard), srv));

        Ok(ServiceServer::new(guard))
    }

    #[instrument(skip(state))]
    pub async fn unregister_service(
        state: &mut ServiceActorState,
        service_name: String,
    ) -> ServiceActorResult<()> {
        trace!("UnregisterService called");

        if let Some((guard, server)) = state.servers.remove(&service_name) {
            guard.upgrade().map(|guard| guard.disarm());

            state
                .master_client
                .unregister_service(&service_name, server.url().as_str())
                .await?;
        }

        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ServiceClientError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Failed to send RPC to client link: {0}")]
    RpcSend(#[from] mpsc::error::SendError<RpcMsg>),
    #[error("Failed to receive RPC response from internal channel: {0}")]
    RpcRecv(#[from] oneshot::error::RecvError),
    #[error("Client link error: {0}")]
    ClientLink(#[from] ClientLinkError),
}

#[derive(Clone)]
pub struct TypedServiceClient<T> {
    inner: ServiceClient,
    _phantom: PhantomData<T>,
}

impl<T> TypedServiceClient<T>
where
    T: rosrust::ServicePair,
{
    pub fn new(client: ServiceClient) -> Self {
        Self {
            inner: client,
            _phantom: PhantomData,
        }
    }

    pub async fn call(&self, request: &T::Request) -> Result<T::Response, ServiceClientError> {
        Ok(T::Response::decode_slice(
            &self.inner.call_raw(request.encode_vec()?).await?,
        )?)
    }
}

#[derive(Clone)]
pub struct ServiceClient {
    rpc_tx: mpsc::UnboundedSender<RpcMsg>,
    _guard: Arc<ServiceClientGuard>,
}

impl ServiceClient {
    pub fn new(rpc_tx: mpsc::UnboundedSender<RpcMsg>, guard: Arc<ServiceClientGuard>) -> Self {
        Self {
            rpc_tx,
            _guard: guard,
        }
    }

    pub async fn call_raw(&self, data: Vec<u8>) -> Result<Vec<u8>, ServiceClientError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.rpc_tx.send(RpcMsg { data, reply_tx })?;
        Ok(reply_rx.await??)
    }
}

pub struct ServiceClientGuard {
    service_name: String,
    armed: AtomicBool,
    actor_ref: ActorRef<ServiceActorMsg>,
}

impl ServiceClientGuard {
    pub fn new(service_name: &str, actor_ref: &ActorRef<ServiceActorMsg>) -> Self {
        Self {
            service_name: service_name.to_string(),
            armed: AtomicBool::new(true),
            actor_ref: actor_ref.clone(),
        }
    }

    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}

impl Drop for ServiceClientGuard {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) {
            if let Err(e) = cast!(
                &self.actor_ref,
                ServiceActorMsg::DropClient {
                    service_name: self.service_name.clone()
                }
            ) {
                warn!(
                    "Failed to trigger client link cleanup: [service: \"{}\", error: \"{e}\"]",
                    self.service_name
                );
            }
        }
    }
}

pub struct ServiceServer {
    _guard: Arc<ServiceServerGuard>,
}

impl ServiceServer {
    pub fn new(guard: Arc<ServiceServerGuard>) -> Self {
        Self { _guard: guard }
    }
}

pub struct ServiceServerGuard {
    service_name: String,
    armed: AtomicBool,
    actor_ref: ActorRef<ServiceActorMsg>,
}

impl ServiceServerGuard {
    pub fn new(service_name: &str, actor_ref: &ActorRef<ServiceActorMsg>) -> Self {
        Self {
            service_name: service_name.to_string(),
            armed: AtomicBool::new(true),
            actor_ref: actor_ref.clone(),
        }
    }

    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}

impl Drop for ServiceServerGuard {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) {
            if let Err(e) = cast!(
                &self.actor_ref,
                ServiceActorMsg::DropService {
                    service_name: self.service_name.clone(),
                }
            ) {
                warn!(
                    "Failed to trigger service server cleanup: [service: \"{}\", error: \"{e}\"]",
                    self.service_name
                );
            }
        }
    }
}
