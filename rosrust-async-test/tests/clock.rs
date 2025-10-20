use std::time::Duration;

use futures::poll;
use rosrust::{msg::rosgraph_msgs::Clock as ClockMsg, Duration as RosDuration, Time};
use tokio::sync::mpsc;

use rosrust_async::clock::Clock;

mod util;
use util::{setup, wait_for_subscriber_connections};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn sleep_and_wake() {
    let (node, _guard) = setup().await;

    let clock_publisher = node
        .publish::<ClockMsg>("/clock", 1, true, false)
        .await
        .unwrap();

    let clock = Clock::new(&node).await.unwrap();

    let mut sleep_future = clock.sleep(RosDuration::from_seconds(5));

    assert!(
        poll!(&mut sleep_future).is_pending(),
        "Sleep future should be pending before deadline"
    );

    // Advance the clock to the future's deadline
    clock_publisher
        .send(&ClockMsg {
            clock: Time { sec: 5, nsec: 0 },
        })
        .unwrap();

    tokio::time::timeout(Duration::from_secs(1), clock.await_init())
        .await
        .expect("Timed out waiting for clock init");

    assert!(
        poll!(&mut sleep_future).is_ready(),
        "Sleep future should be ready after deadline"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn multi_sleep() {
    let (node, _guard) = setup().await;

    let sleep_durations = [1, 3, 5, 7];
    let (done_tx, mut done_rx) = mpsc::channel(sleep_durations.len());

    let clock_publisher = node
        .publish::<ClockMsg>("/clock", 1, false, false)
        .await
        .unwrap();

    let clock = Clock::new(&node).await.unwrap();

    wait_for_subscriber_connections(&node, "/clock", 1, Duration::from_secs(5)).await;

    for duration in sleep_durations {
        let clock = clock.clone();
        let done_tx = done_tx.clone();

        tokio::spawn(async move {
            clock.sleep(RosDuration::from_seconds(duration)).await;
            done_tx.send(duration).await.unwrap();
        });
    }

    for duration in sleep_durations {
        clock_publisher
            .send(&ClockMsg {
                clock: Time::from_seconds(duration as u32),
            })
            .expect("Failed to advance simulated clock");

        let completed_duration = tokio::time::timeout(Duration::from_secs(1), done_rx.recv())
            .await
            .unwrap_or_else(|_| panic!("Timed out waiting for sleep task {duration} to complete"))
            .expect("Task completion channel closed");

        // There should not be any additional completion messages in the channel
        assert!(done_rx.is_empty(), "Multiple sleep tasks completed");

        assert_eq!(
            completed_duration, duration,
            "Incorrect sleep task completed",
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn negative_deadline() {
    let (node, _guard) = setup().await;

    let clock = Clock::new(&node).await.unwrap();
    let sleep_future = clock.sleep(RosDuration::from_seconds(-10));

    assert!(poll!(sleep_future).is_ready());
}
