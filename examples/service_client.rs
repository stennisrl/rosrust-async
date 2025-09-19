//! A ROS1 node that uses the "/add_two_ints" service to sum a pair of user-provided numbers.
//!
//! This example is designed to be run in tandem with `service_server`, which responds to our
//! sum requests with a TwoIntsRes message.

use std::io;

use tracing::{error, info};
use tracing_subscriber::EnvFilter;
rosrust::rosmsg_include!(example_srvs / TwoInts);
use example_srvs::{TwoInts, TwoIntsReq};

use rosrust_async::builder::NodeBuilder;

fn get_number_from_stdin() -> Result<i64, io::Error> {
    loop {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        match input.trim().parse::<i64>() {
            Ok(number) => return Ok(number),
            Err(e) => {
                error!("Failed to parse input: \"{e}\", please try again!");
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("service_client=info".parse()?),
        )
        .init();

    info!("Please enter the first number that you'd like to sum:");
    let first_number = tokio::task::spawn_blocking(|| get_number_from_stdin()).await??;

    info!("Please enter the second number that you'd like to sum:");
    let second_number = tokio::task::spawn_blocking(|| get_number_from_stdin()).await??;

    let node = NodeBuilder::new()
        .name("/service_client")
        .build()
        .await?;

    let client = node
        .service_client::<TwoInts>("/add_two_ints", false)
        .await?;

    let request = TwoIntsReq {
        a: first_number,
        b: second_number,
    };

    let response = client.call(&request).await?;

    info!(
        "Result from /add_two_ints: Sum of {} and {} is {}",
        request.a, request.b, response.sum
    );

    // Shut the node down and clean up any registrations we created with the ROS Master.
    node.shutdown_and_wait(None).await?;

    Ok(())
}
