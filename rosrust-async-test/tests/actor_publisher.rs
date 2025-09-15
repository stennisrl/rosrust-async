mod util;
use rosrust_async::{node::PublisherActorError, NodeError};
use util::{msg::RosString, setup};

use crate::util::msg::TwoIntsReq;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn publish_msg_mismatch() {
    let (node, _guard) = setup().await;

    let _publisher = node
        .publish::<RosString>("/chatter", 1, false, false)
        .await
        .unwrap();

    let second_publisher = node
        .publish::<TwoIntsReq>("/chatter", 1, false, false)
        .await;

    assert!(matches!(
        second_publisher,
        Err(NodeError::Publisher(PublisherActorError::Compatibility(_)))
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn publisher_drop_guard() {
    let (node, _guard) = setup().await;

    let publisher = node
        .publish::<RosString>("/chatter", 1, false, false)
        .await
        .unwrap();

    assert_eq!(
        node.get_publications().await.unwrap().len(),
        1,
        "Publication count mismatch"
    );

    drop(publisher);

    assert!(
        node.get_publications().await.unwrap().is_empty(),
        "Publisher guard did not clean up publication"
    );
}
