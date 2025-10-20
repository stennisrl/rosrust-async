use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use rosrust_async::builder::NodeBuilder;

mod util;
use util::{msg::RosString, setup, wait_for_subscriber_connections, wait_until};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn pub_sub() {
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

    util::test_pubsub(publisher, subscriber).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn pub_sub_loopback() {
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

    util::test_pubsub(publisher, subscriber).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn subscriber_gets_latched_msg() {
    let (node, _guard) = setup().await;

    let message = RosString {
        data: String::from("Hello, world!"),
    };

    let publisher = node
        .publish::<RosString>("/latch_test", 5, true, false)
        .await
        .unwrap();

    publisher.send(&message).unwrap();

    let mut subscriber = node
        .subscribe::<RosString>("/latch_test", 5, false)
        .await
        .unwrap();

    let latched_msg = tokio::time::timeout(Duration::from_secs(5), subscriber.recv())
        .await
        .expect("Timed out waiting for latched message")
        .unwrap();

    assert_eq!(latched_msg, message, "Latched message did not match");
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

    publisher
        .send(&RosString {
            data: String::from("Hello, world!"),
        })
        .unwrap();

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
    .unwrap_or_else(|_| {
        panic!(
            "Timed out waiting for subscribers to check in, {} of {} completed",
            rx_message_count.load(Ordering::Acquire),
            NUM_SUBSCRIBERS
        )
    })
    .unwrap();
}
