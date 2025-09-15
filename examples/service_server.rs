//! A ROS1 node that provides the "/add_two_ints" service which sums pairs of numbers.
//!
//! This example is designed to be run in tandem with `service_client`, which sends a TwoIntsReq
//! message containing two numbers to be summed.

use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
rosrust::rosmsg_include!(std_msgs / TwoInts);
use std_msgs::{TwoInts, TwoIntsRes};

use rosrust_async::builder::NodeBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("service_server=info".parse()?),
        )
        .init();

    let node = NodeBuilder::new()
        .name("/service_server")
        .build()
        .await?;

    info!("Advertising service to /add_two_ints, press Ctrl+C to exit.");

    let _service = node
        .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
            let sum = req.a + req.b;

            info!("Handling sum request: {} + {} = {sum}", req.a, req.b,);

            Ok(TwoIntsRes { sum })
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
