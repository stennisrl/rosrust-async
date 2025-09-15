use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use rosrust_async::node::builder::NodeBuilder;

mod util;
use util::{msg::RosString, setup, wait_for_subscriber_connections};

use crate::util::{
    wait_until,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn publish_to_inline_subscriber() {
    let (node, _guard) = setup().await;

    let second_node = NodeBuilder::new()
        .master_url(node.master_url().clone())
        .name("/rosrust_async_2")
        .build()
        .await
        .unwrap();

    let publisher = node
        .publish::<RosString>("/chatter", 5, false, false)
        .await
        .unwrap();

    let subscriber = second_node
        .subscribe::<RosString>("/chatter", 5, false)
        .await
        .unwrap();

    wait_for_subscriber_connections(&node, "/chatter", 1, Duration::from_secs(5)).await;

    util::test_pubsub(publisher, subscriber, 5).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn publish_loopback() {
    let (node, _guard) = setup().await;

    let publisher = node
        .publish::<RosString>("/chatter", 5, false, false)
        .await
        .unwrap();

    let subscriber = node
        .subscribe::<RosString>("/chatter", 5, false)
        .await
        .unwrap();

    wait_for_subscriber_connections(&node, "/chatter", 1, Duration::from_secs(5)).await;

    util::test_pubsub(publisher, subscriber, 5).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn subscriber_gets_latched_msg() {
    let (node, _guardc) = setup().await;

    let publisher = node
        .publish::<RosString>("/latch_test", 5, true, false)
        .await
        .unwrap();

    publisher
        .send(&RosString {
            data: String::from("cool beans"),
        })
        .unwrap();

    let mut subscriber = node
        .subscribe::<RosString>("/latch_test", 5, false)
        .await
        .unwrap();

    let latched_msg = tokio::time::timeout(Duration::from_secs(5), subscriber.recv())
        .await
        .expect("Timed out waiting for latched message")
        .unwrap();

    assert_eq!(
        latched_msg.data, "cool beans",
        "Latched message did not match"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn publish_to_multiple_subscribers() {
    const NUM_SUBSCRIBERS: usize = 5;

    let (node, _guard) = setup().await;

    let publisher = node
        .publish::<RosString>("/chatter", 5, false, false)
        .await
        .unwrap();

    let rx_message_count = Arc::new(AtomicUsize::new(0));
    let mut subscriber_nodes = Vec::with_capacity(NUM_SUBSCRIBERS);

    for id in 0..NUM_SUBSCRIBERS {
        let sub_node = NodeBuilder::new()
            .name(format!("/subscriber_{id}"))
            .master_url(node.master_url().as_str())
            .build()
            .await
            .unwrap();

        let rx_message_count = rx_message_count.clone();

        sub_node
            .subscribe_callback("/chatter", 5, false, move |_msg: RosString| {
                let rx_message_count = rx_message_count.clone();

                async move {
                    rx_message_count.fetch_add(1, Ordering::Release);
                }
            })
            .await
            .unwrap();

        subscriber_nodes.push(sub_node);
    }

    wait_for_subscriber_connections(&node, "/chatter", NUM_SUBSCRIBERS, Duration::from_secs(5))
        .await;

    let mut message = RosString::default();
    message.data = "Hello, world!".to_string();

    publisher.send(&message).unwrap();

    tokio::time::timeout(
        Duration::from_secs(5),
        wait_until(|| async {
            let msg_count = rx_message_count.load(Ordering::Acquire);
            if msg_count == NUM_SUBSCRIBERS {
                return Ok(Some(()));
            }

            Ok(None)
        }),
    )
    .await
    .expect("Timed out waiting for subscriber messages")
    .unwrap();
}
