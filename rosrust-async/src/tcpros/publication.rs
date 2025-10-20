use std::{collections::HashSet, io, net::SocketAddr, sync::Arc};

use bytes::Bytes;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::{
        broadcast::{self, error::RecvError},
        RwLock,
    },
};
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{debug, error, span, trace, warn, Instrument, Level};

use crate::tcpros::{
    header::{self, HeaderError, PublisherHeader, SubscriberHeader},
    CompatibilityError, Topic,
};

#[derive(thiserror::Error, Debug)]
pub enum PublicationError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("Incompatible headers: {0}")]
    Compatibility(#[from] CompatibilityError),
}

pub struct Publication {
    topic: Topic,
    address: SocketAddr,
    subscriber_ids: Arc<RwLock<HashSet<String>>>,
    data_tx: broadcast::Sender<Bytes>,
    _drop_guard: DropGuard,
}

impl Publication {
    pub async fn new(
        address: SocketAddr,
        topic: &Topic,
        caller_id: &str,
        queue_size: usize,
        tcp_nodelay: bool,
        latching: bool,
    ) -> Result<(broadcast::Sender<Bytes>, Self), PublicationError> {
        let tcp_listener = TcpListener::bind(address).await?;
        let bound_addr = tcp_listener.local_addr()?;

        let header = PublisherHeader {
            caller_id: caller_id.to_string(),
            topic: topic.name.clone(),
            md5sum: topic.spec.md5sum.clone(),
            msg_type: topic.spec.msg_type.clone(),
            msg_definition: topic.spec.msg_definition.clone(),
            latching,
        };

        let header_bytes = header::to_bytes(&header)?;

        let subscriber_ids = Arc::new(RwLock::new(HashSet::<String>::new()));
        let (data_tx, data_rx) = broadcast::channel::<Bytes>(queue_size);
        let cancel_token = CancellationToken::new();

        {
            let subscriber_ids = subscriber_ids.clone();
            let cancel_token = cancel_token.clone();
            let span = span!(
                parent: None,
                Level::DEBUG,
                "publication_listener",
                topic = topic.name.clone(),
                address = bound_addr.to_string(),

            );

            tokio::spawn(
                async move {
                    trace!("Publisher listener task started");

                    Self::listener_task(
                        header,
                        &header_bytes,
                        tcp_nodelay,
                        subscriber_ids,
                        data_rx,
                        tcp_listener,
                        cancel_token,
                    )
                    .await;

                    trace!("Publisher listener task exited");
                }
                .instrument(span),
            );
        }

        let publication = Publication {
            topic: topic.clone(),
            address: bound_addr,
            subscriber_ids,
            data_tx: data_tx.clone(),
            _drop_guard: cancel_token.drop_guard(),
        };

        Ok((data_tx, publication))
    }

    pub fn topic(&self) -> &Topic {
        &self.topic
    }

    pub fn address(&self) -> &SocketAddr {
        &self.address
    }

    pub fn data_sender(&self) -> broadcast::Sender<Bytes> {
        self.data_tx.clone()
    }

    pub async fn subscriber_ids(&self) -> HashSet<String> {
        self.subscriber_ids.read().await.clone()
    }

    pub async fn subscriber_count(&self) -> usize {
        self.subscriber_ids.read().await.len()
    }

    async fn listener_task(
        header: PublisherHeader,
        header_bytes: &[u8],
        tcp_nodelay: bool,
        subscriber_ids: Arc<RwLock<HashSet<String>>>,
        mut data_rx: broadcast::Receiver<Bytes>,
        tcp_listener: TcpListener,
        cancel_token: CancellationToken,
    ) {
        let mut latched_message: Option<Bytes> = None;

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    trace!("Publisher listener task stopped by cancel token.");
                    break;
                }
                message = data_rx.recv(),
                if header.latching => {
                    match message {
                        Ok(m) => {
                            latched_message = Some(m);
                        },
                        Err(RecvError::Lagged(lagged)) => {
                            warn!("Publisher listener task is lagging: [lagged_message_count: {lagged}]");
                        },
                        Err(RecvError::Closed) => {
                            debug!("Internal data channel for publication was closed");
                            break;
                        },
                    }
                }

                connection = tcp_listener.accept() => {
                    let (subscriber_stream, address) = match connection {
                        Ok(connection) => connection,
                        Err(e) => {
                            error!("Failed to accept subscriber connection: {e}");
                            continue;
                        },
                    };

                    let (subscriber_stream, subscriber_header) =
                        match Self::setup_connection(&header, header_bytes, tcp_nodelay, subscriber_stream).await {
                            Ok(connection) => connection,
                            Err(e) => {
                                error!("Failed to set up subscriber connection: {e}");
                                continue;
                            },
                        };

                    {
                        let subscriber_ids = subscriber_ids.clone();
                        let subscriber_id = subscriber_header.caller_id;
                        let subscriber_addr = address.to_string();
                        let latched_message = latched_message.clone();
                        let data_rx = data_rx.resubscribe();
                        let cancel_token = cancel_token.clone();

                        let span =
                            span!(
                                Level::DEBUG,
                                "publisher_task",
                                subscriber_id = subscriber_id,
                                subscriber_addr = subscriber_addr
                            );

                        subscriber_ids.write().await.insert(subscriber_id.clone());

                        tokio::spawn(async move {
                            debug!("Publisher task started");

                            match Self::publisher_task(
                                latched_message,
                                data_rx,
                                subscriber_stream,
                                cancel_token,
                            ).await {
                                Ok(_) => trace!("Publisher task exited"),
                                Err(e) => warn!("Publisher task exited with error: {e}"),
                            }

                            subscriber_ids.write().await.remove(&subscriber_id);
                        }.instrument(span));
                    }
                }
            };
        }
    }

    async fn setup_connection(
        header: &PublisherHeader,
        header_bytes: &[u8],
        tcp_nodelay: bool,
        mut subscriber_stream: TcpStream,
    ) -> Result<(TcpStream, SubscriberHeader), PublicationError> {
        let subscriber_header: SubscriberHeader =
            header::from_async_read(&mut subscriber_stream).await?;

        header::validate_pubsub_compatibility(header, &subscriber_header)?;

        if tcp_nodelay || subscriber_header.tcp_nodelay {
            trace!(
                "Enabling TCP_NODELAY on socket for subscriber \"{}\"",
                subscriber_header.caller_id
            );

            subscriber_stream.set_nodelay(true)?;
        }

        subscriber_stream.write_all(header_bytes).await?;

        Ok((subscriber_stream, subscriber_header))
    }

    async fn publisher_task(
        latched_message: Option<Bytes>,
        mut data_rx: broadcast::Receiver<Bytes>,
        mut subscriber_stream: TcpStream,
        cancel_token: CancellationToken,
    ) -> Result<(), PublicationError> {
        if let Some(last_msg) = latched_message {
            subscriber_stream.write_all(&last_msg).await?;
        }

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    trace!("Publisher task stopped by cancel token.");
                    break;
                }

                msg = data_rx.recv() => {
                    match msg {
                        Ok(msg) => {
                            subscriber_stream.write_all(&msg).await?;
                        },
                        Err(RecvError::Lagged(lagged)) => {
                            warn!("Publisher task is lagging: [lagged_messages: {lagged}]");
                            continue;
                        },
                        Err(RecvError::Closed) => {
                            debug!("Internal data channel for publication was closed");
                            break;
                        },
                    }
                }
            }
        }
        Ok(())
    }
}
