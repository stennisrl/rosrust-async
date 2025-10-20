use std::{io, net::SocketAddr, sync::Arc};

use futures_util::future::BoxFuture;
use rosrust::RosMsg;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{error, span, trace, warn, Instrument, Level};
use url::Url;

use crate::tcpros::{
    self, header::{
        self, HeaderError, ProbeRequestHeader, ProbeResponseHeader, ServiceClientHeader,
        ServiceServerHeader,
    }, read_tcpros_frame, service::{RPC_FAILURE, RPC_SUCCESS}, CompatibilityError, Service
};

pub type CallbackError = Box<dyn std::error::Error + Send + Sync>;
pub type ErasedCallback =
    Arc<dyn Fn(Vec<u8>) -> BoxFuture<'static, Result<Vec<u8>, CallbackError>> + Send + Sync>;

enum RequestKind {
    Probe(ProbeRequestHeader),
    ClientHandshake(ServiceClientHeader),
}

#[derive(thiserror::Error, Debug)]
pub enum ServiceProviderError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    #[error("Could not determine request type")]
    UnknownRequest,
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("Incompatible headers: {0}")]
    Incompatible(#[from] CompatibilityError),
}

pub struct ServiceProvider {
    service: Service,
    address: SocketAddr,
    url: Url,
    _drop_guard: DropGuard,
}

impl ServiceProvider {
    pub async fn new(
        address: SocketAddr,
        service: &Service,
        caller_id: &str,
        callback: ErasedCallback,
    ) -> Result<Self, ServiceProviderError> {
        let tcp_listener = TcpListener::bind(address).await?;
        let bound_addr = tcp_listener.local_addr()?;
        let url = Url::parse(&format!(
            "rosrpc://{}:{}",
            bound_addr.ip(),
            bound_addr.port()
        ))?;

        let cancel_token = CancellationToken::new();

        let probe_header_bytes = header::to_bytes(&ProbeResponseHeader {
            caller_id: caller_id.to_string(),
            md5sum: service.spec.md5sum.clone(),
            msg_type: service.spec.msg_type.clone(),
        })?;

        let server_header_bytes = header::to_bytes(&ServiceServerHeader {
            caller_id: caller_id.to_string(),
        })?;

        let span = span!(
            parent: None,
            Level::DEBUG,
            "service_server_listener",
            service = service.name.clone(),
            address = bound_addr.to_string(),
        );

        {   
            let service = service.clone();
            let cancel_token = cancel_token.clone();

            tokio::spawn(
                async move {
                    Self::listener_task(
                        service,
                        tcp_listener,
                        callback,
                        &probe_header_bytes,
                        &server_header_bytes,
                        cancel_token,
                    )
                    .await;
                    trace!("Listener task exited");
                }
                .instrument(span),
            );
        }
        Ok(Self {
            service: service.clone(),
            address: bound_addr,
            url,
            _drop_guard: cancel_token.drop_guard(),
        })
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn address(&self) -> &SocketAddr {
        &self.address
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    async fn listener_task(
        service: Service,
        tcp_listener: TcpListener,
        callback: ErasedCallback,
        probe_header_bytes: &[u8],
        server_header_bytes: &[u8],
        cancel_token: CancellationToken,
    ) {
        trace!("Listener task started");

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    trace!("Service server task stopped by cancel token.");
                    break;
                }

                connection = tcp_listener.accept() => {
                    let (client_stream, address) = match connection {
                        Ok(connection) => connection,
                        Err(e) => {
                            error!("Failed to accept client connection: {e}");
                            continue;
                        },
                    };

                    if let Err(e) =
                        Self::handle_request(
                            &service,
                            probe_header_bytes,
                            server_header_bytes,
                            &callback,
                            &address,
                            client_stream,
                            &cancel_token,
                        ).await {
                        warn!("Failed to handle client request: {e}");
                    };
                }
            }
        }
    }

    async fn handle_request(
        service: &Service,
        probe_header_bytes: &[u8],
        server_header_bytes: &[u8],
        callback: &ErasedCallback,
        client_addr: &SocketAddr,
        mut client_stream: TcpStream,
        cancel_token: &CancellationToken,
    ) -> Result<(), ServiceProviderError> {
        let header_data = read_tcpros_frame(&mut client_stream).await?;

        let request_kind = match header::from_bytes(&header_data) {
            Ok(client) => RequestKind::ClientHandshake(client),
            Err(_req_err) => match header::from_bytes(&header_data) {
                Ok(probe) => RequestKind::Probe(probe),
                Err(_probe_err) => return Err(ServiceProviderError::UnknownRequest),
            },
        };

        match request_kind {
            RequestKind::Probe(_probe) => {
                client_stream.write_all(probe_header_bytes).await?;
            }
            RequestKind::ClientHandshake(client) => {
                header::validate_client_compatibility(service, &client)?;
                
                client_stream.write_all(server_header_bytes).await?;

                let callback = callback.clone();
                let cancel_token = cancel_token.clone();

                let span = span!(
                    Level::DEBUG,
                    "rpc_handler",
                    client_id = client.caller_id,
                    client_addr = client_addr.to_string(),
                    persistent = client.persistent.to_string(),
                );

                tokio::spawn(
                    async move {
                        match Self::rpc_task(
                            client_stream,
                            client.persistent,
                            callback,
                            cancel_token,
                        )
                        .await
                        {
                            Ok(_) => trace!("RPC handler task exited"),
                            Err(e) => error!("RPC handler task exited with error: {e}"),
                        }
                    }
                    .instrument(span),
                );
            }
        }
        Ok(())
    }

    async fn rpc_task(
        mut client_stream: TcpStream,
        persistent: bool,
        callback: ErasedCallback,
        cancel_token: CancellationToken,
    ) -> Result<(), io::Error> {
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    trace!("RPC handler task stopped by cancel token.");
                    break;
                }

                data = tcpros::read_tcpros_frame(&mut client_stream) => {
                    let data = data?;
                    match callback(data).await {
                        Ok(response) => {
                            client_stream.write_u8(RPC_SUCCESS).await?;
                            client_stream.write_all(&response).await?;
                        },
                        Err(e) => {
                            warn!("Service callback failed: {}", e);

                            client_stream.write_u8(RPC_FAILURE).await?;

                            let mut error_msg_bytes = Vec::new();
                            e.to_string().encode(&mut error_msg_bytes)?;

                            client_stream.write_all(&error_msg_bytes).await?;
                        },
                    }
                }
            }

            if !persistent {
                trace!("Non-persistent connection, RPC handler task exiting");
                break;
            }
        }
        Ok(())
    }
}
