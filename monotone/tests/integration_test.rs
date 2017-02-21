extern crate monotone;
extern crate serde_json;
#[macro_use]
extern crate error_chain;
extern crate hyper;
extern crate rand;

#[cfg(feature="aws")]
extern crate rusoto;

#[cfg(feature="aws")]
mod aws {


use monotone::*;
use monotone::string::*;
use monotone::aws::counter::*;
use monotone::aws::queue::*;
use self::error::*;
use rusoto::*;
use rusoto::dynamodb::*;
use std::env;
use std::time::Duration;
use std::str::FromStr;
use std::ops::Deref;
use hyper;
use rand;

mod error {
    use monotone;
    use rusoto;
    use serde_json;

    error_chain! {
        foreign_links {
            Tls(rusoto::TlsError);
            Credentials(rusoto::CredentialsError);
            ParseRegion(rusoto::ParseRegionError);
            Json(serde_json::Error);
        }

        links {
            MonotoneAws(monotone::aws::error::Error, monotone::aws::error::ErrorKind);
        }

        errors {
        }
    }
}

/*
interesting test cases:
for each initial condition [no row in db, row in db]:
  Counter::get()
  Counter::next()
  Counter::next() with interleaved write to db
  Queue::get_ticket()
  Queue::get_tickets()
  Queue::join()
  Queue::join() with interleaved write to db
  Queue::leave()
  Queue::leave() with interleaved write to db
  Queue::leave() with later items repositioned
*/

pub fn client() -> Result<DynamoDbClient<DefaultCredentialsProvider,hyper::client::Client>> {
    let provider = DefaultCredentialsProvider::new()?;
    let region = Region::from_str("eu-west-1")?;
    let client = DynamoDbClient::new(default_tls_client()?, provider, region);
    Ok(client)
}

pub fn table_name() -> String {
    return s("Counters");
}

pub fn build_id() -> Option<String> {
    let build_id = env::var("TRAVIS_BUILD_ID").map(|v| Some(v)).unwrap_or(None);
    let build_number = env::var("TRAVIS_BUILD_NUMBER").map(|v| Some(v)).unwrap_or(None);

    build_id.and_then(|id| {
        build_number.and_then(|num| {
            Some(format!("{}-{}", id, num))
        })
    })
}

pub fn counter_id() -> String {
    let build_id = build_id().unwrap_or(s("no_build_id"));
    let random = rand::random::<u64>();
    format!("it-counter-{}-{}", build_id, random)
}

pub fn queue_id() -> String {
    let build_id = build_id().unwrap_or(s("no_build_id"));
    let random = rand::random::<u64>();
    format!("it-queue-{}-{}", build_id, random)
}

pub fn retry_time() -> Duration {
    Duration::from_secs(1)
}

pub struct TestCounter {
    pub counter: Counter<DefaultCredentialsProvider, hyper::client::Client>,
}

impl TestCounter {
    pub fn new() -> TestCounter {
        TestCounter {
            counter: Counter::new(client().expect("client"), table_name(), counter_id(), retry_time())
        }
    }
}

impl Drop for TestCounter {
    fn drop(&mut self) {
        self.counter.remove().expect("remove");
    }
}

impl Deref for TestCounter {
    type Target = Counter<DefaultCredentialsProvider, hyper::client::Client>;

    fn deref(&self) -> &Self::Target {
        &self.counter
    }
}

pub struct TestQueue {
    pub queue: Queue<DefaultCredentialsProvider, hyper::client::Client>,
}

impl TestQueue {
    pub fn new() -> TestQueue {
        TestQueue {
            queue: Queue::new(client().expect("client"), table_name(), queue_id(), retry_time())
        }
    }
}

impl Drop for TestQueue {
    fn drop(&mut self) {
        self.queue.remove().expect("remove");
    }
}

impl Deref for TestQueue {
    type Target = Queue<DefaultCredentialsProvider, hyper::client::Client>;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

#[test]
pub fn test_counter_no_row_get() {
    let c = TestCounter::new();

    let v = c.get_value().expect("get");
    assert_eq!(v, 0);
}

#[test]
pub fn test_counter_row_get() {
    let c = TestCounter::new();

    let v = c.get_value().expect("get");
    assert_eq!(v, 0);

    let v = c.next_value().expect("next");
    assert_eq!(v, 1);

    let v = c.get_value().expect("get");
    assert_eq!(v, 1);
}

#[test]
pub fn test_counter_row_next() {
    let c = TestCounter::new();

    let v = c.next_value().expect("next");
    assert_eq!(v, 1);

    let v = c.next_value().expect("next");
    assert_eq!(v, 2);

    let v = c.get_value().expect("get");
    assert_eq!(v, 2);
}

#[test]
pub fn test_counter_no_row_interleaved_write_next() {

}

#[test]
pub fn test_counter_row_interleaved_write_next() {

}

#[test]
pub fn test_queue_no_row_get() {
    let q = TestQueue::new();

    assert!(q.get_ticket("foo").is_err());
}

#[test]
pub fn test_queue_row_get() {
    let q = TestQueue::new();

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let (ft, tok2) = q.get_ticket("foo").expect("get");
    assert_eq!(ft, 1);
    assert_eq!(tok2, tok);
}

#[test]
pub fn test_queue_no_row_get_all() {
    let q = TestQueue::new();

    let (ft, toks) = q.get_tickets().expect("get all");
    assert_eq!(ft, 0);
    assert_eq!(toks, vec![]);
}

#[test]
pub fn test_queue_row_get_all() {
    let q = TestQueue::new();

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let (ft, toks) = q.get_tickets().expect("get all");
    assert_eq!(ft, 1);
    assert_eq!(toks, vec![tok]);
}

#[test]
pub fn test_queue_same_row_join() {
    let q = TestQueue::new();

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let (ft, tok2) = q.get_ticket("foo").expect("get");
    assert_eq!(ft, 1);
    assert_eq!(tok2, tok);
}

#[test]
pub fn test_queue_different_row_join() {
    let q = TestQueue::new();

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let (ft, tok) = q.join_queue(s("bar")).expect("join");
    assert_eq!(ft, 2);
    assert_eq!(&tok.process_id, "bar");
    assert_eq!(tok.counter, 2);
    assert_eq!(tok.position, 1);
}

#[test]
pub fn test_queue_no_row_interleaved_write_join() {

}

#[test]
pub fn test_queue_row_interleaved_write_join() {

}

#[test]
pub fn test_queue_no_row_leave() {
    let q = TestQueue::new();

    assert!(q.leave_queue("foo").is_err());
}

#[test]
pub fn test_queue_row_leave() {
    let q = TestQueue::new();

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let ft = q.leave_queue("foo").expect("leave");
    assert_eq!(ft, 2);
}

#[test]
pub fn test_queue_no_row_interleaved_write_leave() {

}

#[test]
pub fn test_queue_row_interleaved_write_leave() {

}

#[test]
pub fn test_queue_rows_leave() {
#[test]
pub fn test_queue_row_leave() {
    let q = TestQueue::new();

    let (ft, tok) = q.join_queue(s("foo")).expect("join");
    assert_eq!(ft, 1);
    assert_eq!(&tok.process_id, "foo");
    assert_eq!(tok.counter, 1);
    assert_eq!(tok.position, 0);

    let (ft, tok) = q.join_queue(s("bar")).expect("join");
    assert_eq!(ft, 2);
    assert_eq!(&tok.process_id, "bar");
    assert_eq!(tok.counter, 2);
    assert_eq!(tok.position, 1);

    let ft = q.leave_queue("foo").expect("leave");
    assert_eq!(ft, 3);
    
    let (ft, tok) = q.get_ticket("bar").expect("get");
    assert_eq!(ft, 3);
    assert_eq!(&tok.process_id, "bar");
    assert_eq!(tok.counter, 2);
    assert_eq!(tok.position, 0);
}
}

}