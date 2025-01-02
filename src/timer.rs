use core::convert::Infallible;

use embassy_time::{Duration, Instant, Timer};
use futures::Future;

pub struct LoraTimer {
    start: Instant,
}
impl LoraTimer {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
}
impl Default for LoraTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl ::lorawan::device::timer::Timer for LoraTimer {
    type Error = Infallible;

    fn reset(&mut self) {
        self.start = Instant::now();
    }

    type AtFuture<'a> = impl Future<Output = ()> + 'a;

    fn at<'a>(&self, millis: u64) -> Result<Self::AtFuture<'a>, Self::Error> {
        let start = self.start;
        let fut = Timer::at(start + Duration::from_millis(millis));
        Ok(fut)
    }
}
