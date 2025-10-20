use rosrust_async::{node::SubscriberActorError, NodeError};

mod util;
use util::{
    msg::{RosString, TwoIntsReq},
    setup,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn second_subscribe_fails_on_msg_mismatch() {
    let (node, _guard) = setup().await;

    let _subscriber = node
        .subscribe::<RosString>("/chatter", 1, false)
        .await
        .unwrap();

    let _second_subscriber = node.subscribe::<TwoIntsReq>("/chatter", 1, false).await;

    assert!(matches!(
        _second_subscriber,
        Err(NodeError::Subscriber(SubscriberActorError::Compatibility(
            _
        )))
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn subscriber_drop_guard() {
    let (node, _guard) = setup().await;

    let subscriber = node
        .subscribe::<RosString>("/chatter", 1, false)
        .await
        .unwrap();

    assert_eq!(
        node.get_subscriptions().await.unwrap().len(),
        1,
        "Subscription count mismatch"
    );

    drop(subscriber);

    assert_eq!(
        node.get_subscriptions().await.unwrap().len(),
        0,
        "Subscriber drop guard did not clean up Subscription"
    );
}
