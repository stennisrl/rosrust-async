use rosrust_async::{
    builder::NodeBuilder, tcpros::service::client::ClientLinkError, ServiceClientError,
};

mod util;
use util::{
    msg::{TwoInts, TwoIntsReq, TwoIntsRes},
    setup, test_sum,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn call_service() {
    let (node, _guard) = setup().await;
    let second_node = NodeBuilder::new()
        .master_url(node.master_url().clone())
        .name("/rosrust_async_2")
        .build()
        .await
        .unwrap();

    let client = node
        .service_client::<TwoInts>("/add_two_ints", true)
        .await
        .unwrap();

    let _server = second_node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
            Ok(TwoIntsRes { sum: req.a + req.b })
        })
        .await
        .unwrap();

    test_sum(&client, 0, 10).await;
    test_sum(&client, 9, 10).await;
    test_sum(&client, 100, -200).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn call_service_loopback() {
    let (node, _guard) = setup().await;

    let client = node
        .service_client::<TwoInts>("/add_two_ints", false)
        .await
        .unwrap();

    let _server = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
            Ok(TwoIntsRes { sum: req.a + req.b })
        })
        .await
        .unwrap();

    test_sum(&client, 0, 10).await;
    test_sum(&client, 9, 10).await;
    test_sum(&client, 100, -200).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn service_error_propagates_to_client() {
    let error_string = String::from("something went wrong!");

    let (node, _guard) = setup().await;

    let client = node
        .service_client::<TwoInts>("/add_two_ints", false)
        .await
        .unwrap();

    let cb_error_string = error_string.clone();
    let _server = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", move |_req| {
            let error_string = cb_error_string.clone();
            async move { Err(error_string.into()) }
        })
        .await
        .unwrap();

    let response = client.call(&TwoIntsReq { a: 5, b: 5 }).await;

    assert!(
        matches!(
            response,
            Err(ServiceClientError::ClientLink(ClientLinkError::RpcFailure(
                ref error
            )))
            if error == &error_string,

        ),
        "Expected RpcFailure with msg \"{}\" but got {:?}",
        error_string,
        response
    );
}
