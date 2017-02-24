
#![recursion_limit = "100"]

#[macro_use]
extern crate error_chain;
extern crate rand;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate log;
#[cfg(feature = "aws")]
extern crate rusoto;

pub mod error;
pub mod local;
#[cfg(feature = "aws")]
pub mod aws;
pub mod string;
pub mod time;

use std::result;
use std::collections::BTreeMap;

pub trait MonotonicCounter {
    type Error;

    fn get_value(&self) -> result::Result<u64, Self::Error>;

    fn next_value(&self) -> result::Result<u64, Self::Error>;
}

pub type FencingToken = u64;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct Ticket {
    pub process_id: String,
    pub counter: u64,
    pub position: usize,
    pub tags: BTreeMap<String, String>,
}

impl Ticket {
    pub fn new(process_id: String, counter: u64, position: usize, tags: BTreeMap<String, String>) -> Ticket {
        Ticket {
            process_id: process_id,
            counter: counter,
            position: position,
            tags: tags,
        }
    }
}

pub trait MonotonicQueue {
    type Error;

    fn join_queue<T>(&self, process_id: String, tags: T) -> result::Result<(FencingToken, Ticket), Self::Error> where T: Into<Option<BTreeMap<String, String>>>;

    fn leave_queue(&self, process_id: &str) -> result::Result<FencingToken, Self::Error>;

    fn get_ticket(&self, process_id: &str) -> result::Result<(FencingToken, Ticket), Self::Error>;

    fn get_tickets(&self) -> result::Result<(FencingToken, Vec<Ticket>), Self::Error>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
