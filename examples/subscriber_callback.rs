//! A ROS1 node that echoes messages sent to the "/chatter" topic.
//!
//! This example is functionally identical to `subscriber`, the only difference being that
//! this one showcases the `[rosrust_async::Node::subscribe_callback]` method.
//!
//! As with the `subscriber` example, you can use either the `publisher` example or the
//! [rostopic CLI tool](http://wiki.ros.org/rostopic) to send messages to the subscription.
//!
//! The `RosString` message used in this example is identical to
//! [std_msgs/String](https://docs.ros.org/en/melodic/api/std_msgs/html/msg/String.html),
//! so using `rostopic` is reasonably straightforward:
//!
//! ```bash
//! rostopic pub -r 1 /chatter std_msgs/String "Hello from rostopic!"
//! ```
//!
//! If you do not have a local ROS installation but want to try out `rostopic`,
//! you can do something similar to what is outlined in the readme:
//!
//! ```bash
//! docker run -it --rm --network host ros:noetic-ros-core bash
//!
//! # Then, from inside the interactive shell:
//! rostopic pub -r 1 /chatter std_msgs/String "Hello from rostopic!"
//! ```
//!

use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

rosrust::rosmsg_include!(std_msgs / String);
use std_msgs::String as RosString;

use rosrust_async::builder::NodeBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("subscriber_callback=info".parse()?),
        )
        .init();

    let node = NodeBuilder::new()
        .name("/subscriber_callback")
        .build()
        .await?;

    info!("Subscribing to /chatter, press Ctrl+C to exit.");

    // The cancel token & join handle allow for fine-grained control of the
    // callback if desired. For this example, we use `[rosrust_async::Node::shutdown_and_wait]`
    // which cleans up the internal Subscription channel leading to the callback exiting gracefully.
    let (_cancel_token, _handle) = node
        .subscribe_callback("/chatter", 1, false, |msg: RosString| async move {
            info!("Received message: {msg:?}");
        })
        .await?;

    match signal::ctrl_c().await {
        Ok(_) => info!("Ctrl+C detected, exiting"),
        Err(e) => error!("Failed to install signal handler: {e}"),
    };

    // Shut the node down and clean up any registrations we created with the ROS Master.
    node.shutdown_and_wait(None).await?;

    Ok(())
}
