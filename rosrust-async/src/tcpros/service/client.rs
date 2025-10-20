use std::{io, net::SocketAddr};

use rosrust::RosMsg;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{
        mpsc::{self, UnboundedReceiver},
        oneshot,
    },
};
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{debug, error, span, trace, warn, Instrument, Level};
use url::Url;

use crate::{
    tcpros::{
        self,
        header::{
            self, HeaderError, ProbeRequestHeader, ProbeResponseHeader, ServiceClientHeader,
            ServiceServerHeader,
        },
        service::{RPC_FAILURE, RPC_SUCCESS},
        CompatibilityError, Service,
    },
    xmlrpc::{MasterClientError, RosMasterClient},
};

#[derive(thiserror::Error, Debug)]
pub enum ClientLinkError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Header(#[from] header::HeaderError),
    #[error("Master client error: {0}")]
    MasterClient(#[from] MasterClientError),
    #[error("Failed to parse service url: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("Failed to resolve service URL to a socket address: {service_url}")]
    ServiceResolution {
        service_url: Url,
        #[source]
        error: io::Error,
    },
    #[error("No addresses available for service URL: {0}")]
    NoServiceAddresses(Url),
    #[error("Incompatible headers: {0}")]
    Incompatible(#[from] CompatibilityError),
    #[error("RPC response contained invalid status code: {0}")]
    InvalidStatusCode(u8),
    #[error("RPC response indicated failure: {0}")]
    RpcFailure(String),
}

#[derive(Debug)]
pub struct RpcMsg {
    pub data: Vec<u8>,
    pub reply_tx: oneshot::Sender<Result<Vec<u8>, ClientLinkError>>,
}

struct ConnectionState {
    address: SocketAddr,
    tcp_stream: TcpStream,
    server_header: ServiceServerHeader,
}

pub struct ServiceClientLink {
    service: Service,
    rpc_tx: mpsc::UnboundedSender<RpcMsg>,
    _drop_guard: DropGuard,
}

impl ServiceClientLink {
    pub async fn new(
        service: &Service,
        caller_id: &str,
        persistent: bool,
        master_client: RosMasterClient,
    ) -> Result<(mpsc::UnboundedSender<RpcMsg>, Self), HeaderError> {
        let (rpc_tx, rpc_rx) = mpsc::unbounded_channel::<RpcMsg>();
        let cancel_token = CancellationToken::new();

        let probe_header_bytes = header::to_bytes(&ProbeRequestHeader {
            caller_id: caller_id.to_string(),
            service: service.name.clone(),
            md5sum: service.spec.md5sum.clone(),
            probe: true,
        })?;

        let client_header_bytes = header::to_bytes(&ServiceClientHeader {
            caller_id: caller_id.to_string(),
            service: service.name.clone(),
            md5sum: service.spec.md5sum.clone(),
            msg_type: service.spec.msg_type.clone(),
            persistent,
        })?;

        let span = span!(parent: None, Level::DEBUG, "service_client_task", service = service.name, persistent = persistent);

        {
            let service = service.clone();
            let cancel_token = cancel_token.clone();

            tokio::spawn(
                async move {
                    Self::rpc_task(
                        service,
                        master_client,
                        persistent,
                        &client_header_bytes,
                        &probe_header_bytes,
                        rpc_rx,
                        cancel_token,
                    )
                    .await;

                    trace!("Service client task exited");
                }
                .instrument(span),
            );
        }

        Ok((
            rpc_tx.clone(),
            Self {
                rpc_tx,
                service: service.clone(),
                _drop_guard: cancel_token.drop_guard(),
            },
        ))
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn rpc_sender(&self) -> mpsc::UnboundedSender<RpcMsg> {
        self.rpc_tx.clone()
    }

    async fn probe_service(
        address: &SocketAddr,
        probe_header_bytes: &[u8],
    ) -> Result<ProbeResponseHeader, ClientLinkError> {
        let mut tcp_stream = TcpStream::connect(address).await?;
        tcp_stream.write_all(probe_header_bytes).await?;

        Ok(header::from_async_read(&mut tcp_stream).await?)
    }

    async fn connect_to_service(
        service: &Service,
        master_client: &RosMasterClient,
        client_header_bytes: &[u8],
        probe_header_bytes: &[u8],
    ) -> Result<ConnectionState, ClientLinkError> {
        trace!("Attempting to resolve address for service");
        let service_url = Url::parse(&master_client.lookup_service(&service.name).await?)?;

        trace!("Found service at url \"{service_url}\"");

        let address = service_url
            .socket_addrs(|| None)
            .map_err(|error| ClientLinkError::ServiceResolution {
                service_url: service_url.clone(),
                error,
            })?
            .first()
            .cloned()
            .ok_or(ClientLinkError::NoServiceAddresses(service_url.clone()))?;

        trace!("Resolved service url \"{service_url}\" to \"{address}\"");

        let probe_response = Self::probe_service(&address, probe_header_bytes).await?;
        header::validate_server_compatibility(service, &probe_response)?;

        let mut tcp_stream = TcpStream::connect(address).await?;
        tcp_stream.write_all(client_header_bytes).await?;

        let server_header: ServiceServerHeader = header::from_async_read(&mut tcp_stream).await?;

        Ok(ConnectionState {
            address,
            tcp_stream,
            server_header,
        })
    }

    async fn rpc_task(
        service: Service,
        master_client: RosMasterClient,
        persistent: bool,
        client_header_bytes: &[u8],
        probe_header_bytes: &[u8],
        mut rpc_rx: UnboundedReceiver<RpcMsg>,
        cancel_token: CancellationToken,
    ) {
        debug!("Service client task started");
        let mut connection_state: Option<ConnectionState> = None;

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    trace!("Service client task stopped by cancel token.");
                    break;
                }

                rpc_msg = rpc_rx.recv() => {
                    let Some(RpcMsg { data, reply_tx }) = rpc_msg else {
                        debug!("Internal data channel for service client was closed");
                        break;
                    };

                    let handler_result = async {
                        let mut connection = match connection_state.take() {
                            Some(state) => {
                                trace!("Reusing persistent connection for RPC");
                                state
                            },
                            None => {
                                Self::connect_to_service(
                                    &service,
                                    &master_client,
                                    client_header_bytes,
                                    probe_header_bytes,
                                ).await?
                            },
                        };

                        let span =
                            span!(
                                Level::DEBUG,
                                "rpc_handler",
                                server_id = connection.server_header.caller_id,
                                server_addr = connection.address.to_string()
                            );

                        let rpc_result = Self::handle_rpc(&data, &mut connection.tcp_stream).instrument(span).await;

                        if persistent {
                            connection_state = Some(connection);
                        }

                        rpc_result
                    }.await;

                    if reply_tx.send(handler_result).is_err() {
                        warn!("Failed to send RPC result to client handle, channel closed");
                    }
                }
            }
        }
    }

    async fn handle_rpc(
        request: &[u8],
        mut tcp_stream: &mut TcpStream,
    ) -> Result<Vec<u8>, ClientLinkError> {
        tcp_stream.write_all(request).await?;

        match tcp_stream.read_u8().await? {
            RPC_SUCCESS => Ok(tcpros::read_tcpros_frame(&mut tcp_stream).await?),
            RPC_FAILURE => {
                let error_msg_bytes = tcpros::read_tcpros_frame(&mut tcp_stream).await?;
                let error_msg = String::decode(error_msg_bytes.as_slice())?;
                let error = ClientLinkError::RpcFailure(error_msg);
                warn!("{error}");

                Err(error)
            }
            mystery_code => {
                let error = ClientLinkError::InvalidStatusCode(mystery_code);
                error!("{error}");

                Err(error)
            }
        }
    }
}
