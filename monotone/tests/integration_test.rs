extern crate monotone;
extern crate serde_json;
#[macro_use]
extern crate error_chain;
extern crate hyper;

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
use hyper;

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
    env::var("TRAVIS_BUILD_ID").map(|v| Some(v)).unwrap_or(None)
}

pub fn counter_id() -> String {
    let build_id = build_id().unwrap_or(s("no_build_id"));
    format!("it-counter-{}", build_id)
}

pub fn queue_id() -> String {
    let build_id = build_id().unwrap_or(s("no_build_id"));
    format!("it-queue-{}", build_id)
}

pub fn retry_time() -> Duration {
    Duration::from_secs(1)
}

#[test]
pub fn it_works() {
    println!("testing..");
}

#[test]
pub fn test_counter_no_row_get() {
    let c = Counter::new(client().expect("client"), table_name(), counter_id(), retry_time());
    let v = c.get_value().expect("get");
    assert_eq!(v, 0);
}

#[test]
pub fn test_counter_row_get() {
    let c = Counter::new(client().expect("client"), table_name(), counter_id(), retry_time());

    let v = c.next_value().expect("next");
    assert_eq!(v, 1);

    let v = c.get_value().expect("get");
    assert_eq!(v, 1);
}

#[test]
pub fn test_counter_no_row_next() {

}

#[test]
pub fn test_counter_row_next() {

}

#[test]
pub fn test_counter_no_row_interleaved_write_next() {

}

#[test]
pub fn test_counter_row_interleaved_write_next() {

}

#[test]
pub fn test_queue_no_row_get() {

}

#[test]
pub fn test_queue_row_get() {

}

#[test]
pub fn test_queue_no_row_get_all() {

}

#[test]
pub fn test_queue_row_get_all() {

}

#[test]
pub fn test_queue_no_row_join() {

}

#[test]
pub fn test_queue_same_row_join() {

}

#[test]
pub fn test_queue_different_row_join() {

}

#[test]
pub fn test_queue_no_row_interleaved_write_join() {

}

#[test]
pub fn test_queue_row_interleaved_write_join() {

}

#[test]
pub fn test_queue_no_row_leave() {

}

#[test]
pub fn test_queue_row_leave() {

}

#[test]
pub fn test_queue_no_row_interleaved_write_leave() {

}

#[test]
pub fn test_queue_row_interleaved_write_leave() {

}

#[test]
pub fn test_queue_rows_leave() {

}

}