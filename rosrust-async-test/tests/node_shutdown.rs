use std::time::Duration;

use rosrust_async::xmlrpc::{RosMasterClient, RosSlaveClient};

mod util;
use util::{msg::RosString, setup};

use crate::util::msg::{TwoInts, TwoIntsRes};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn node_handles_api_shutdown() {
    let (node, _guard) = setup().await;
    let slave_api = RosSlaveClient::new(&node.url(), "/integration_tests");

    slave_api
        .shutdown("Testing shutdown functionality")
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), node.shutdown_complete())
        .await
        .expect("Node did not shut down");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn shutdown_releases_resources() {
    let (node, _guard) = setup().await;
    let node_name = node.name().to_string();

    let master_api = RosMasterClient::new(node.master_url(), &node_name, node.url().to_string());

    let _publisher = node
        .publish::<RosString>("/chatter", 1, false, false)
        .await
        .unwrap();

    let _subscriber = node
        .subscribe::<RosString>("/chatter", 1, false)
        .await
        .unwrap();

    let _server = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
            Ok(TwoIntsRes { sum: req.a + req.b })
        })
        .await
        .unwrap();

    assert!(
        master_api
            .get_system_state()
            .await
            .unwrap()
            .is_publishing(&node_name, "/chatter"),
        "Node not publishing to topic"
    );

    assert!(
        master_api
            .get_system_state()
            .await
            .unwrap()
            .is_subscribed(&node_name, "/chatter"),
        "Node not subscribed to topic"
    );

    assert!(
        master_api
            .get_system_state()
            .await
            .unwrap()
            .is_providing_service(&node_name, "/add_two_ints"),
        "Node not providing service"
    );

    node.shutdown_and_wait(None).await.unwrap();

    assert!(
        !master_api
            .get_system_state()
            .await
            .unwrap()
            .is_subscribed(&node_name, "/chatter"),
        "Node shutdown did not clean up subscription"
    );

    assert!(
        !master_api
            .get_system_state()
            .await
            .unwrap()
            .is_publishing(&node_name, "/chatter"),
        "Node shutdown did not clean up publication"
    );

    assert!(
        !master_api
            .get_system_state()
            .await
            .unwrap()
            .is_providing_service(&node_name, "/add_two_ints"),
        "Node shutdown did not clean up service"
    );
}
