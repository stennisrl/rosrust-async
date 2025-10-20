use std::time::Duration;

use tokio::sync::mpsc::{self};

mod util;
use util::{
    create_rosrust_node,
    msg::{RosString, TwoInts, TwoIntsReq, TwoIntsRes},
    setup, test_sum, wait_for_publisher_connections, wait_for_subscriber_connections,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn publish_to_rosrust() {
    let (node, _guard) = setup().await;
    let rosrust = create_rosrust_node(node.master_url(), "rosrust_subscriber");

    let publisher = node
        .publish::<RosString>("/chatter", 5, false, false)
        .await
        .unwrap();

    let (subscriber_tx, subscriber_rx) = mpsc::unbounded_channel();
    let _subscriber = rosrust
        .subscribe::<RosString, _>("/chatter", 5, move |data| subscriber_tx.send(data).unwrap())
        .unwrap();

    wait_for_subscriber_connections(&node, "/chatter", 1, Duration::from_secs(5)).await;

    util::test_pubsub(publisher, subscriber_rx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn subscribe_to_rosrust() {
    let (node, _guard) = setup().await;
    let rosrust = create_rosrust_node(node.master_url(), "rosrust_publisher");

    let publisher = rosrust.publish::<RosString>("/chatter", 100).unwrap();
    let subscriber = node
        .subscribe::<RosString>("/chatter", 5, false)
        .await
        .unwrap();

    wait_for_publisher_connections(&node, "/chatter", 1, Duration::from_secs(5)).await;

    util::test_pubsub(publisher, subscriber).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn call_rosrust_service() {
    let (node, _guard) = setup().await;
    let rosrust = create_rosrust_node(node.master_url(), "rosrust_service");

    let _service = rosrust
        .service::<TwoInts, _>("/add_two_ints", move |req| {
            Ok(TwoIntsRes { sum: req.a + req.b })
        })
        .unwrap();

    let client = node
        .service_client::<TwoInts>("/add_two_ints", false)
        .await
        .unwrap();

    test_sum(&client, 0, 10).await;
    test_sum(&client, 9, 10).await;
    test_sum(&client, 100, -200).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn advertise_service_to_rosrust() {
    let (node, _guard) = setup().await;
    let rosrust = create_rosrust_node(node.master_url(), "rosrust_client");

    let _service = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
            Ok(TwoIntsRes { sum: req.a + req.b })
        })
        .await
        .unwrap();
    
    let client = rosrust.client::<TwoInts>("/add_two_ints").unwrap();
    let client_task = std::thread::spawn(move || client.req(&TwoIntsReq { a: 10, b: 15 }));

    tokio::time::timeout(Duration::from_secs(5), async move {
        while !client_task.is_finished() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        client_task.join()
    })
    .await
    .expect("Timed out waiting for RPC")
    .unwrap()
    .unwrap()
    .unwrap();
}
