//! A ROS1 node that repeatedly publishes a message to the "/chatter" topic.
//!
//! This example is designed to run in tandem with `subscriber`, however
//! it will also work with the [rostopic CLI tool](http://wiki.ros.org/rostopic).
//!
//! The `RosString` message used in this example is identical to
//! [std_msgs/String](https://docs.ros.org/en/melodic/api/std_msgs/html/msg/String.html),
//! so using `rostopic` is reasonably straightforward:
//!
//! ```bash
//! rostopic echo /chatter
//! ```
//!
//! If you do not have a local ROS installation but want to try out `rostopic`,
//! you can do something similar to what is outlined in the readme:
//!
//! ```bash
//! docker run -it --rm --network host ros:noetic-ros-core bash
//!
//! # Then, from inside the interactive shell:
//! rostopic echo /chatter
//! ```
//!

use std::time::Duration;

use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

rosrust::rosmsg_include!(std_msgs / String);
use std_msgs::String as RosString;

use rosrust_async::builder::NodeBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("publisher=info".parse()?))
        .without_time()
        .init();

    let node = NodeBuilder::new()
        .name("/publisher_node")
        .build()
        .await?;

    let publisher = node
        .publish::<RosString>("/chatter", 1, false, false)
        .await?;

    let mut interval = tokio::time::interval(Duration::from_secs(1));

    let msg = RosString {
        data: String::from("Hello world!"),
    };

    info!("Publishing to /chatter, press Ctrl+C to exit.");

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = publisher.send(&msg) {
                    error!("Failed to publish message: {e}");
                    break;
                }
            }
            _ = signal::ctrl_c() => {
                info!("Ctrl+C detected, exiting");
                break;
            }
        }
    }

    // Shut the node down and clean up any registrations we created with the ROS Master.
    node.shutdown_and_wait(None).await?;

    Ok(())
}
