
use std::time::Duration;
use std::ops::Add;
use rand;
use rand::distributions::{IndependentSample, Range};

pub trait Jitter {
    fn jitter(&self, millis: u64) -> Duration;
}

impl Jitter for Duration {
    fn jitter(&self, millis: u64) -> Duration {
        let mut rng = rand::thread_rng();
        let range = Range::new(0, millis);
        let rand = range.ind_sample(&mut rng);

        self.add(Duration::from_millis(rand))
    }
}