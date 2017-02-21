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

#[cfg(test)]
mod tests {
    use ::*;
    use super::*;

    #[test]
    pub fn test_counter_get() {
        let c = Counter::new();
        assert_eq!(0, c.get_value().expect("get"));
    }

    #[test]
    pub fn test_counter_next() {
        let c = Counter::new();
        assert_eq!(1, c.next_value().expect("next"));
        assert_eq!(1, c.get_value().expect("get"));

        assert_eq!(2, c.next_value().expect("next"));
        assert_eq!(2, c.get_value().expect("get"));
    }
}