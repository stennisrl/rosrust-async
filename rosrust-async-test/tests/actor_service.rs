mod util;

use rosrust_async::{node::ServiceActorError, NodeError};
use util::{
    msg::{DudService, TwoInts},
    setup,
};

use crate::util::msg::TwoIntsRes;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn second_client_fails_on_mismatch() {
    let (node, _guard) = setup().await;

    let _client = node
        .service_client::<TwoInts>("/add_two_ints", false)
        .await
        .unwrap();

    let second_client = node
        .service_client::<DudService>("/add_two_ints", false)
        .await;

    assert!(matches!(
        second_client,
        Err(NodeError::Service(ServiceActorError::Compatibility(_)))
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn second_server_fails_on_mismatch() {
    let (node, _guard) = setup().await;

    let _server = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |_| async move {
            Ok(TwoIntsRes::default())
        })
        .await
        .unwrap();

    let second_server = node
        .advertise_service::<DudService, _, _>("/add_two_ints", |_| async move { Ok(0) })
        .await;

    assert!(matches!(
        second_server,
        Err(NodeError::Service(ServiceActorError::Compatibility(_)))
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn server_drop_guard() {
    let (node, _guard) = setup().await;

    let server = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
            Ok(TwoIntsRes { sum: req.a + req.b })
        })
        .await
        .unwrap();

    assert_eq!(
        node.get_services().await.unwrap().len(),
        1,
        "Service count mismatch"
    );

    drop(server);

    assert!(
        node.get_services().await.unwrap().is_empty(),
        "Service guard did not clean up server"
    );
}
