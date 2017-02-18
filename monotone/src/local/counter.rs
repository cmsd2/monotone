use std::sync::{Arc, Mutex};

use ::*;
use ::error::*;

#[derive(Debug, Clone)]
pub struct Counter {
    counter: Arc<Mutex<u64>>,
}

impl Counter {
    pub fn new() -> Counter {
        Counter {
            counter: Arc::new(Mutex::new(0))
        }
    }
}

impl MonotonicCounter for Counter {
    type Error = Error;
    
    fn get_value(&self) -> Result<u64> {
        let counter = self.counter.lock().unwrap();

        Ok(*counter)
    }

    fn next_value(&self) -> Result<u64> {
        let mut counter = self.counter.lock().unwrap();

        *counter += 1;

        Ok(*counter)
    }
}