use std::{future::Future, pin::Pin, sync::Arc, task::Poll};

use diatomic_waker::WakeSink;
use pin_project::pin_project;
use rosrust::Time;
use tracing::error;

use crate::clock::{ClockState, TimerEntry};

#[pin_project]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SleepFuture {
    deadline: Time,
    clock_state: Arc<ClockState>,
    waker: Option<WakeSink>,
}

impl SleepFuture {
    pub fn new(deadline: Time, clock_state: Arc<ClockState>) -> Self {
        SleepFuture {
            deadline,
            clock_state,
            waker: None,
        }
    }
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if this.clock_state.time.get() >= *this.deadline
            || this.clock_state.timer_tx.is_closed()
        {
            return Poll::Ready(());
        }

        match this.waker {
            Some(waker) => waker.register(cx.waker()),
            None => {
                let mut waker = WakeSink::new();
                waker.register(cx.waker());

                let timer_entry = TimerEntry {
                    deadline: *this.deadline,
                    waker: waker.source(),
                };

                if let Err(e) = this.clock_state.timer_tx.send(timer_entry) {
                    error!("Failed to register timer: {e}");
                    cx.waker().wake_by_ref();
                    return Poll::Ready(());
                }

                *this.waker = Some(waker);
            }
        }

        Poll::Pending
    }
}
