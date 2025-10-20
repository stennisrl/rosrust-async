use std::{
    collections::{HashMap, HashSet},
    io,
    sync::Arc,
};

use bytes::Bytes;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpStream, ToSocketAddrs},
    sync::{broadcast, RwLock},
};
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{debug, error, span, trace, warn, Instrument, Level};

use crate::tcpros::{
    self,
    header::{self, HeaderError, PublisherHeader, SubscriberHeader},
    CompatibilityError, Topic,
};

#[derive(thiserror::Error, Debug)]
pub enum SubscriptionError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("Incompatible headers: {0}")]
    Compatibility(#[from] CompatibilityError),
}

pub struct Subscription {
    topic: Topic,
    header: SubscriberHeader,
    header_bytes: Vec<u8>,
    publisher_addrs: Arc<RwLock<HashSet<String>>>,
    latched_msgs: Arc<RwLock<HashMap<String, Bytes>>>,
    data_tx: broadcast::Sender<Bytes>,
    cancel_token: CancellationToken,
    _data_rx: broadcast::Receiver<Bytes>,
    _drop_guard: DropGuard,
}

impl Subscription {
    pub fn new(
        topic: &Topic,
        caller_id: &str,
        queue_size: usize,
        tcp_nodelay: bool,
    ) -> Result<(broadcast::Receiver<Bytes>, Self), SubscriptionError> {
        let header = SubscriberHeader {
            caller_id: caller_id.to_string(),
            topic: topic.name.clone(),
            md5sum: topic.spec.md5sum.clone(),
            msg_type: topic.spec.msg_type.clone(),
            msg_definition: topic.spec.msg_definition.clone(),
            tcp_nodelay,
        };

        let header_bytes = header::to_bytes(&header)?;

        let (data_tx, data_rx) = broadcast::channel(queue_size);
        let cancel_token = CancellationToken::new();

        let subscription = Subscription {
            topic: topic.clone(),
            header,
            header_bytes,
            publisher_addrs: Arc::default(),
            latched_msgs: Arc::default(),
            data_tx: data_tx.clone(),
            cancel_token: cancel_token.clone(),
            _data_rx: data_rx.resubscribe(),
            _drop_guard: cancel_token.drop_guard(),
        };

        Ok((data_rx, subscription))
    }

    pub fn topic(&self) -> &Topic {
        &self.topic
    }

    pub fn data_receiver(&self) -> broadcast::Receiver<Bytes> {
        self.data_tx.subscribe()
    }

    pub async fn latched_msgs(&self) -> Vec<Bytes> {
        self.latched_msgs.read().await.values().cloned().collect()
    }

    pub async fn publisher_addrs(&self) -> HashSet<String> {
        self.publisher_addrs.read().await.clone()
    }

    pub async fn publisher_count(&self) -> usize {
        self.publisher_addrs.read().await.len()
    }

    pub async fn add_publisher<A>(
        &mut self,
        publisher_addr: &str,
        channel_addr: A,
    ) -> Result<(), SubscriptionError>
    where
        A: ToSocketAddrs,
    {
        if self.publisher_addrs.read().await.contains(publisher_addr) {
            debug!(
                "Publisher at \"{publisher_addr}\" has already been added to this subscription."
            );

            return Ok(());
        }

        let mut publisher_stream = TcpStream::connect(&channel_addr).await?;
        publisher_stream.write_all(&self.header_bytes).await?;

        let publisher_header: PublisherHeader =
            header::from_async_read(&mut publisher_stream).await?;
        let publisher_id = publisher_header.caller_id.clone();

        header::validate_pubsub_compatibility(&publisher_header, &self.header)?;

        if publisher_header.latching {
            debug!("Publisher \"{publisher_id}\" is set to latching mode.");
        }

        {
            let publisher_addr = publisher_addr.to_string();
            let publisher_addrs = self.publisher_addrs.clone();
            let latched_msgs = self.latched_msgs.clone();
            let data_tx = self.data_tx.clone();
            let cancel_token = self.cancel_token.clone();

            let span = span!(
                parent: None,
                Level::DEBUG,
                "subscriber_task",
                topic = self.topic.name.clone(),
                publisher_id = publisher_id,
                publisher_addr = publisher_addr,
            );

            self.publisher_addrs
                .write()
                .await
                .insert(publisher_addr.clone());

            tokio::spawn(
                async move {
                    debug!("Subscriber task started");

                    match Self::subscriber_task(
                        publisher_header,
                        latched_msgs.clone(),
                        data_tx,
                        publisher_stream,
                        cancel_token,
                    )
                    .await
                    {
                        Ok(_) => trace!("Subscriber task exited"),
                        Err(e) => warn!("Subscriber task exited with error: {e}"),
                    }

                    latched_msgs.write().await.remove(&publisher_addr);
                    publisher_addrs.write().await.remove(&publisher_addr);
                }
                .instrument(span),
            );
        }

        Ok(())
    }

    async fn subscriber_task(
        publisher_header: PublisherHeader,
        latched_msgs: Arc<RwLock<HashMap<String, Bytes>>>,
        data_tx: broadcast::Sender<Bytes>,
        mut publisher_stream: TcpStream,
        cancel_token: CancellationToken,
    ) -> Result<(), io::Error> {
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() =>{
                    trace!("Subscriber task stopped by cancel token.");
                    break;
                }

                data = tcpros::read_tcpros_frame(&mut publisher_stream) => {
                    let data = Bytes::from(data?);

                    if data_tx.send(data.clone()).is_err() {
                        debug!("Internal data channel for subscription was closed");
                        break;
                    }

                    if publisher_header.latching {
                        latched_msgs
                            .write()
                            .await
                            .insert(publisher_header.caller_id.clone(), data);
                    }
                }
            }
        }

        Ok(())
    }
}
