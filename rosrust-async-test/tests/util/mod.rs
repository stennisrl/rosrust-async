use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddrV4},
    sync::mpsc::RecvError,
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc::UnboundedReceiver};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use ros_core_rs::core::Master;
use url::Url;

use rosrust::Publisher;
use rosrust_async::{
    node::{
        builder::NodeBuilder, Node, NodeError, PublisherError, SubscriberError, TypedPublisher,
        TypedSubscriber,
    },
    TypedServiceClient,
};

pub mod msg;
use msg::RosString;

use crate::util::msg::{TwoInts, TwoIntsReq};

pub async fn setup() -> (Node, WorkerGuard) {
    if std::env::var("NEXTEST").is_err() {
        panic!("Integration tests must be run using cargo nextest!");
    }

    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(non_blocking)
        .finish();

    tracing::subscriber::set_global_default(subscriber).unwrap();

    let master_addr = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
        .local_addr()
        .unwrap();

    let ros_master = Master::new(&master_addr);

    tokio::spawn(async move {
        ros_master.serve().await.unwrap();
    });

    let node = NodeBuilder::new()
        .master_url(format!(
            "http://{}:{}",
            master_addr.ip(),
            master_addr.port()
        ))
        .build()
        .await
        .unwrap();

    (node, guard)
}

#[allow(dead_code)]
pub fn create_rosrust_node(master_url: &Url, name: &str) -> rosrust::api::Ros {
    let master_url = master_url.to_string();
    let name = name.to_string();

    temp_env::with_vars(
        [
            ("ROS_MASTER_URI", Some(master_url)),
            ("ROS_IP", Some(Ipv4Addr::LOCALHOST.to_string())),
        ],
        move || rosrust::api::Ros::new(&name).unwrap(),
    )
}

pub async fn wait_until<F, T, Fut>(mut condition: F) -> Result<T, NodeError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>, NodeError>>,
{
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    
    loop {
        if let Some(val) = condition().await? {
            return Ok(val);
        }

        interval.tick().await;
    }
}

#[allow(dead_code)]
pub async fn wait_for_subscriber_connections(
    node: &Node,
    topic_name: &str,
    subscriber_count: usize,
    timeout: Duration,
) {
    tokio::time::timeout(
        timeout,
        wait_until(|| async {
            if let Some(connected_subscribers) =
                node.get_connected_subscriber_ids(topic_name).await?
            {
                if connected_subscribers.len() == subscriber_count {
                    return Ok(Some(connected_subscribers));
                }
            }

            Ok(None)
        }),
    )
    .await
    .expect(&format!(
        "Timed out waiting for {subscriber_count} subscriber(s) to connect"
    ))
    .unwrap();
}

#[allow(dead_code)]
pub async fn wait_for_publisher_connections(
    node: &Node,
    topic_name: &str,
    publisher_count: usize,
    timeout: Duration,
) {
    tokio::time::timeout(
        timeout,
        wait_until(|| async {
            if let Some(connected_publishers) =
                node.get_connected_publisher_urls(topic_name).await?
            {
                if connected_publishers.len() == publisher_count {
                    return Ok(Some(connected_publishers));
                }
            }

            Ok(None)
        }),
    )
    .await
    .expect(&format!(
        "Timed out waiting for {publisher_count} publisher(s) to connect"
    ))
    .unwrap();
}

#[allow(dead_code)]
pub async fn test_sum(client: &TypedServiceClient<TwoInts>, a: i64, b: i64) {
    let sum = tokio::time::timeout(Duration::from_secs(5), client.call(&TwoIntsReq { a, b }))
        .await
        .expect("Timed out waiting for RPC result")
        .unwrap()
        .sum;

    assert_eq!(a + b, sum);
}

#[async_trait::async_trait]
pub trait MessageSender {
    type Error: std::error::Error;
    async fn send_message(&self, message: &RosString) -> Result<(), Self::Error>;
}

#[async_trait::async_trait]
pub trait MessageReceiver {
    type Error: std::error::Error;
    async fn recv_message(&mut self) -> Result<RosString, Self::Error>;
}

#[allow(dead_code)]
pub async fn test_pubsub<P, S>(publisher: P, mut subscriber: S, iterations: usize)
where
    P: MessageSender,
    S: MessageReceiver,
{
    let base_message = "Test message";
    let delimiter = ':';

    for msg_id in 0..iterations {
        let msg = RosString {
            data: format!("{base_message}{delimiter}{msg_id}"),
        };

        publisher.send_message(&msg).await.unwrap();

        let recv_msg = tokio::time::timeout(Duration::from_secs(5), subscriber.recv_message())
            .await
            .expect("Timed out waiting for message")
            .expect("Failed to recv message");

        let (msg, id) = recv_msg
            .data
            .split_once(delimiter)
            .expect("Message did not contain delimiter");

        assert_eq!(
            msg, base_message,
            "Message did not contain \"{base_message}\""
        );

        let recv_id: usize = id.parse().expect("Failed to parse message ID");

        assert_eq!(msg_id, recv_id, "Message IDs did not match");
    }
}

#[async_trait::async_trait]
impl MessageSender for Publisher<RosString> {
    type Error = rosrust::error::Error;

    async fn send_message(&self, msg: &RosString) -> Result<(), Self::Error> {
        self.send(msg.clone())
    }
}

#[async_trait::async_trait]
impl MessageReceiver for UnboundedReceiver<RosString> {
    type Error = RecvError;

    async fn recv_message(&mut self) -> Result<RosString, Self::Error> {
        self.recv().await.ok_or(RecvError)
    }
}

#[async_trait::async_trait]
impl MessageSender for TypedPublisher<RosString> {
    type Error = PublisherError;

    async fn send_message(&self, msg: &RosString) -> Result<(), Self::Error> {
        self.send(msg)
    }
}

#[async_trait::async_trait]

impl MessageReceiver for TypedSubscriber<RosString> {
    type Error = SubscriberError;

    async fn recv_message(&mut self) -> Result<RosString, Self::Error> {
        self.recv().await
    }
}
