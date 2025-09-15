use rosrust::Duration;

use crate::clock::{Clock, EncodedTime};

pub struct Rate {
    clock: Clock,
    interval: Duration,
    next: EncodedTime,
}

impl Rate {
    pub fn new(clock: Clock, interval: Duration) -> Self {
        let now = clock.now();
        Self {
            clock,
            interval,
            next: EncodedTime::new(now),
        }
    }

    pub async fn sleep(&self) {
        let deadline = self.next.get() + self.interval;
        self.next.set(deadline);
        self.clock.sleep_until(deadline).await
    }
}
