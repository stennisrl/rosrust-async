use std::collections::HashMap;

use rosrust_async::{builder::NodeBuilder, xmlrpc::RosSlaveClient, NodeError};

mod util;
use util::setup;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn set_get_delete() {
    let (node, _guard) = setup().await;
    node.set_param("/test_param", 1234).await.unwrap();

    assert_eq!(
        node.get_param("/test_param").await.unwrap(),
        Some(1234),
        "Parameter value on server does not match"
    );

    node.delete_param("/test_param").await.unwrap();

    assert_eq!(
        node.get_param::<i32>("/test_param").await.unwrap(),
        None,
        "Parameter still exists after being deleted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn param_search() {
    let (node, _guard) = setup().await;
    node.set_param("/test_param", 1234).await.unwrap();

    let search_result = node.search_param("/test_param").await.unwrap();

    assert!(search_result.is_some(), "Search returned no results");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn cached_param() {
    let (node, _guard) = setup().await;
    let node_client = RosSlaveClient::new(node.url(), "/node_client");

    assert_eq!(
        node.get_param_cached::<i32>("/test_param").await.unwrap(),
        None,
        "Parameter existed before being set"
    );

    node_client.param_update("/test_param", 1234).await.unwrap();

    assert_eq!(
        node.get_param_cached("/test_param").await.unwrap(),
        Some(1234),
        "Parameter not present in cache"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn cache_cleared_on_param_delete() {
    let (node, _guard) = setup().await;
    let second_node = NodeBuilder::new()
        .name("/second_node")
        .master_url(node.master_url().to_string())
        .build()
        .await
        .unwrap();

    assert_eq!(
        node.get_param_cached::<i32>("/test_param").await.unwrap(),
        None,
        "Parameter existed before being set"
    );

    node.set_param("/test_param", 1234).await.unwrap();

    second_node.delete_param("/test_param").await.unwrap();

    assert_eq!(
        node.get_param_cached::<i32>("/test_param").await.unwrap(),
        None,
        "Parameter was not cleared from cache"
    );
}
