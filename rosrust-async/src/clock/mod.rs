use std::{
    cmp::Reverse,
    collections::{binary_heap::PeekMut, BinaryHeap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use diatomic_waker::WakeSource;
use rosrust::{msg::rosgraph_msgs::Clock as ClockMsg, Duration, Time};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{debug, error, trace, warn};

use crate::{
    node::{Node, NodeError, SubscriberError},
    TypedSubscriber,
};

pub mod rate;
pub mod sleep;

use sleep::SleepFuture;

#[derive(Debug)]
struct TimerEntry {
    deadline: Time,
    waker: WakeSource,
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deadline.cmp(&other.deadline)
    }
}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for TimerEntry {}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline
    }
}

type TimerQueue = BinaryHeap<Reverse<TimerEntry>>;

#[derive(Default)]
struct EncodedTime {
    time: AtomicU64,
}

impl EncodedTime {
    fn encode_time(time: Time) -> u64 {
        ((time.sec as u64) << 32) | time.nsec as u64
    }

    fn decode_time(encoded_time: u64) -> Time {
        Time {
            sec: (encoded_time >> 32) as u32,
            nsec: encoded_time as u32,
        }
    }

    pub fn new(time: Time) -> Self {
        EncodedTime {
            time: AtomicU64::new(Self::encode_time(time)),
        }
    }

    pub fn get(&self) -> Time {
        Self::decode_time(self.time.load(Ordering::Acquire))
    }

    pub fn set(&self, now: Time) {
        self.time.store(Self::encode_time(now), Ordering::Release);
    }
}

#[derive(Clone)]
pub struct Clock {
    state: Arc<ClockState>,
    _drop_guard: Arc<DropGuard>,
}

pub struct ClockState {
    time: EncodedTime,
    timer_tx: mpsc::UnboundedSender<TimerEntry>,
}

impl Clock {
    pub async fn new(node: &Node) -> Result<Self, NodeError> {
        let clock_subscriber = node.subscribe("/clock", 1, false).await?;

        Ok(Self::from_subscriber(clock_subscriber).await)
    }

    pub async fn from_subscriber(mut clock_subscriber: TypedSubscriber<ClockMsg>) -> Self {
        let cancel_token = CancellationToken::new();

        let mut timers = TimerQueue::new();
        let (timer_tx, mut timer_rx) = mpsc::unbounded_channel();

        let clock_state = Arc::new(ClockState {
            time: EncodedTime::default(),
            timer_tx,
        });

        {
            let cancel_token = cancel_token.clone();
            let clock_state = clock_state.clone();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = cancel_token.cancelled() => {
                            trace!("Clock callback stopped by cancel token");
                            break;
                        }

                        timer_entry = timer_rx.recv() => {
                            match timer_entry {
                                Some(timer_entry) => timers.push(Reverse(timer_entry)),
                                None => {
                                    debug!("Internal timer channel for clock was closed");
                                    break;
                                },
                            }
                        }

                        msg = clock_subscriber.recv() => {
                            match msg {
                                Ok(msg) => Self::on_tick(msg, &clock_state, &mut timers),
                                Err(SubscriberError::Recv(RecvError::Lagged(_))) => {},
                                Err(SubscriberError::Recv(RecvError::Closed)) => {
                                    debug!("Internal data channel for clock was closed");
                                    break;
                                },
                                Err(e) => {
                                    error!("Failed to recv clock message: {e}");
                                    break;
                                },
                            }
                        }
                    }
                }

                timer_rx.close();

                for timer in timers
                    .into_iter()
                    .map(|reverse| reverse.0)
                    .chain(std::iter::from_fn(|| timer_rx.try_recv().ok()))
                {
                    timer.waker.notify();
                }
            });
        }

        Self {
            state: clock_state,
            _drop_guard: Arc::new(cancel_token.drop_guard()),
        }
    }

    fn on_tick(clock_msg: ClockMsg, clock_state: &Arc<ClockState>, timers: &mut TimerQueue) {
        let current_ts = clock_msg.clock;
        let previous_ts = clock_state.time.get();

        clock_state.time.set(current_ts);

        if current_ts < previous_ts {
            warn!("Simulated ROS clock jumped backwards: [previous: {previous_ts:?}, current: {current_ts:?}]");
        }

        loop {
            match timers.peek_mut() {
                Some(timer) if current_ts >= timer.0.deadline => {
                    timer.0.waker.notify();
                    PeekMut::pop(timer);
                }

                _ => break,
            }
        }
    }

    /// Wait until the `/clock` subscriber starts receiving timestamps.
    ///
    /// It is highly recommended to call this prior to using any time-dependent methods like [`Self::sleep`].
    ///
    /// Until timestamp messages are received, [`Self::now`] will return `Time { sec: 0, nsec: 0 }`
    /// (the clock's initial state), and `SleepFuture` objects will sleep indefinitely.
    pub async fn await_init(&self) {
        self.sleep_until(Time::from_nanos(1)).await;
    }

    /// Returns the current simulated time.
    ///
    /// This method uses atomic operations and is safe to call concurrently.
    pub fn now(&self) -> Time {
        self.state.time.get()
    }

    /// Waits until `deadline` is reached.
    ///
    /// The returned future will complete immediately if the deadline is in the past.
    pub fn sleep_until(&self, deadline: Time) -> SleepFuture {
        SleepFuture::new(deadline, self.state.clone())
    }

    /// Waits until `duration` has elapsed.
    ///
    /// The returned future will complete immediately if the duration is negative or 0.
    pub fn sleep(&self, duration: Duration) -> SleepFuture {
        self.sleep_until(self.now() + duration.max(Duration::new()))
    }

    /// Convenience method for sleeping a number of seconds.
    ///
    /// The returned future will complete immediately if `secs` is negative or 0.
    pub fn sleep_seconds(&self, secs: i32) -> SleepFuture {
        self.sleep(Duration::from_seconds(secs))
    }
}
