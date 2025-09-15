use rosrust_async::xmlrpc::RosSlaveClient;

mod util;
use util::setup;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn set_get_delete() {
    let (node, _guard) = setup().await;
    node.set_param("/test_param", 1234).await.unwrap();

    assert!(
        node.has_param("/test_param").await.unwrap(),
        "Parameter was not created/set"
    );

    assert_eq!(
        node.get_param("/test_param").await.unwrap(),
        Some(1234),
        "Parameter value on server did not match"
    );

    node.delete_param("/test_param").await.unwrap();

    assert!(
        !node.has_param("/test_param").await.unwrap(),
        "Parameter existed post-delete"
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
pub async fn types_are_checked() {
    let (node, _guard) = setup().await;
    node.set_param("/test_param", 1234).await.unwrap();

    assert!(
        node.get_param::<bool>("/test_param").await.is_err(),
        "Parameter type was ignored"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn cache_gets_updated() {
    let (node, _guard) = setup().await;

    let node_client = RosSlaveClient::new(node.url(), "/node_client");
    let param_value = String::from("Hello, world!");

    // Inform the node that our param of interest was modified,
    // even though it does not actually exist on the parameter server.
    // This will store `test_param` in the node's param cache.
    node_client
        .param_update("/test_param", &param_value)
        .await
        .unwrap();

    assert_eq!(
        node.get_param_cached::<String>("/test_param")
            .await
            .unwrap(),
        Some(param_value)
    )
}
