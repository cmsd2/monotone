//! Counter and queue spec scenarios through the async traits, plus
//! concurrency and hand-interleaved conflict tests.

use std::collections::BTreeSet;
use std::sync::Arc;

use rand::SeedableRng;
use rand::rngs::SmallRng;

use super::*;
use crate::core::machine::Effect;
use crate::core::row::{CounterRow, Row};

fn tags(pairs: &[(&str, &str)]) -> Tags {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ---- counter ----

#[tokio::test]
async fn fresh_counter_reads_zero() {
    let c = Store::new().counter("c");
    assert_eq!(c.get_value().await, Ok(0));
}

#[tokio::test]
async fn increment_returns_new_value_and_get_agrees() {
    let c = Store::new().counter("c");
    assert_eq!(c.next_value().await, Ok(1));
    assert_eq!(c.get_value().await, Ok(1));
    assert_eq!(c.next_value().await, Ok(2));
    assert_eq!(c.get_value().await, Ok(2));
}

#[tokio::test]
async fn counter_clones_and_handles_share_state() {
    let store = Store::new();
    let a = store.counter("c");
    let b = a.clone();
    let c = Counter::new(&store, "c");
    a.next_value().await.unwrap();
    b.next_value().await.unwrap();
    assert_eq!(c.get_value().await, Ok(2));
    assert_eq!(store.counter("other").get_value().await, Ok(0));
}

#[tokio::test]
async fn counter_remove_resets_and_tolerates_missing() {
    let c = Store::new().counter("c");
    for _ in 0..5 {
        c.next_value().await.unwrap();
    }
    c.remove().await.unwrap();
    assert_eq!(c.get_value().await, Ok(0));
    c.remove().await.unwrap();
}

#[tokio::test]
async fn counter_on_queue_row_is_wrong_type_and_vice_versa() {
    let store = Store::new();
    store.queue("x").join_queue("p", None).await.unwrap();
    let wrong = Error::WrongType {
        expected: "COUNTER",
        found: "QUEUE".into(),
    };
    assert_eq!(store.counter("x").get_value().await, Err(wrong.clone()));
    assert_eq!(store.counter("x").next_value().await, Err(wrong));

    store.counter("y").next_value().await.unwrap();
    let wrong = Error::WrongType {
        expected: "QUEUE",
        found: "COUNTER".into(),
    };
    assert_eq!(store.queue("y").get_tickets().await, Err(wrong.clone()));
    assert_eq!(store.queue("y").join_queue("p", None).await, Err(wrong));
}

// ---- queue ----

#[tokio::test]
async fn empty_queue_lists_token_zero() {
    let q = Store::new().queue("q");
    assert_eq!(q.get_tickets().await, Ok((0, vec![])));
}

#[tokio::test]
async fn join_ticket_shape_and_numbering() {
    let q = Store::new().queue("q");
    let (token, foo) = q.join_queue("foo", None).await.unwrap();
    assert_eq!(token, 1);
    assert_eq!(
        foo,
        Ticket {
            process_id: "foo".into(),
            counter: 1,
            position: 0,
            tags: Tags::new()
        }
    );

    let (token, bar) = q.join_queue("bar", None).await.unwrap();
    assert_eq!((token, bar.counter, bar.position), (2, 2, 1));
}

#[tokio::test]
async fn token_increments_once_per_write() {
    let q = Store::new().queue("q");
    assert_eq!(q.join_queue("foo", None).await.unwrap().0, 1);
    assert_eq!(q.join_queue("bar", None).await.unwrap().0, 2);
    assert_eq!(q.leave_queue("foo").await, Ok(3));
}

#[tokio::test]
async fn read_returns_current_token_and_same_ticket() {
    let q = Store::new().queue("q");
    let joined = q.join_queue("foo", None).await.unwrap();
    assert_eq!(q.get_ticket("foo").await, Ok(joined));
}

#[tokio::test]
async fn join_with_tags_round_trips_through_every_read() {
    let q = Store::new().queue("q");
    let t = tags(&[("role", "leader")]);
    let (_, ticket) = q.join_queue("foo", t.clone()).await.unwrap();
    assert_eq!(ticket.tags, t);
    assert_eq!(q.get_ticket("foo").await.unwrap().1.tags, t);
    assert_eq!(q.get_tickets().await.unwrap().1[0].tags, t);
}

#[tokio::test]
async fn rejoin_is_idempotent() {
    let q = Store::new().queue("q");
    let first = q.join_queue("foo", None).await.unwrap();
    assert_eq!(
        q.join_queue("foo", tags(&[("new", "tag")])).await,
        Ok(first)
    );
    let (token, tickets) = q.get_tickets().await.unwrap();
    assert_eq!(token, 1);
    assert_eq!(tickets.len(), 1);
}

#[tokio::test]
async fn leave_shifts_later_positions_and_keeps_counters() {
    let q = Store::new().queue("q");
    q.join_queue("foo", None).await.unwrap();
    let (_, bar) = q.join_queue("bar", None).await.unwrap();
    q.leave_queue("foo").await.unwrap();
    let (token, now) = q.get_ticket("bar").await.unwrap();
    assert_eq!(token, 3);
    assert_eq!((now.position, now.counter), (0, bar.counter));
}

#[tokio::test]
async fn leave_unknown_process_is_not_found_and_token_unchanged() {
    let q = Store::new().queue("q");
    q.join_queue("foo", None).await.unwrap();
    assert_eq!(
        q.leave_queue("nobody").await,
        Err(Error::NotFound("nobody".into()))
    );
    assert_eq!(q.get_tickets().await.unwrap().0, 1);
}

#[tokio::test]
async fn leave_and_lookup_on_missing_queue_are_not_found() {
    let q = Store::new().queue("q");
    assert_eq!(
        q.leave_queue("foo").await,
        Err(Error::NotFound("foo".into()))
    );
    assert_eq!(
        q.get_ticket("foo").await,
        Err(Error::NotFound("foo".into()))
    );
    assert_eq!(q.get_tickets().await, Ok((0, vec![])));
}

#[tokio::test]
async fn list_after_joins_is_in_position_order() {
    let q = Store::new().queue("q");
    q.join_queue("foo", None).await.unwrap();
    q.join_queue("bar", None).await.unwrap();
    let names: Vec<_> = q
        .get_tickets()
        .await
        .unwrap()
        .1
        .into_iter()
        .map(|t| (t.process_id, t.position))
        .collect();
    assert_eq!(names, vec![("foo".to_string(), 0), ("bar".to_string(), 1)]);
}

#[tokio::test]
async fn queue_remove_resets_everything() {
    let q = Store::new().queue("q");
    q.join_queue("foo", None).await.unwrap();
    q.join_queue("bar", None).await.unwrap();
    q.remove().await.unwrap();
    assert_eq!(q.get_tickets().await, Ok((0, vec![])));
    assert_eq!(q.join_queue("baz", None).await.unwrap().1.counter, 1);
    Store::new().queue("never").remove().await.unwrap();
}

// ---- concurrency ----

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_increments_and_joins_never_collide() {
    let store = Store::new();
    let counter = store.counter("c");
    let queue = store.queue("q");

    let increments: Vec<_> = (0..100)
        .map(|_| {
            let c = counter.clone();
            tokio::spawn(async move { c.next_value().await.unwrap() })
        })
        .collect();
    let joins: Vec<_> = (0..50)
        .map(|i| {
            let q = queue.clone();
            tokio::spawn(async move { q.join_queue(&format!("p{i}"), None).await.unwrap() })
        })
        .collect();

    let mut values = BTreeSet::new();
    for h in increments {
        assert!(values.insert(h.await.unwrap()), "duplicate counter value");
    }
    assert_eq!(values, (1..=100).collect());
    assert_eq!(counter.get_value().await, Ok(100));

    let mut counters = BTreeSet::new();
    for h in joins {
        let (_, ticket) = h.await.unwrap();
        assert!(counters.insert(ticket.counter), "duplicate queue counter");
    }
    assert_eq!(counters, (1..=50).collect());

    let (token, tickets) = queue.get_tickets().await.unwrap();
    assert_eq!(token, 50);
    let positions: BTreeSet<_> = tickets.iter().map(|t| t.position).collect();
    assert_eq!(positions, (0..50).collect());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let q = Arc::new(Store::new().queue("q"));
    let fut = q.join_queue("a", None);
    assert_send(&fut);
    fut.await.unwrap();
}

// ---- hand-interleaved conflict ----

#[test]
fn second_writer_conflicts_rereads_and_reflects_the_winner() {
    let store = Store::new();
    let id = "c";
    let mut a = ops::next_value(RetryPolicy::default(), SmallRng::seed_from_u64(1));
    let mut b = ops::next_value(RetryPolicy::default(), SmallRng::seed_from_u64(2));

    let read = |s: &Store| s.read(id).map(|i| CounterRow::decode(&i).unwrap());

    // Both start and read the same missing row.
    assert_eq!(a.start(), Ok(Step::Effect(Effect::Read)));
    assert_eq!(b.start(), Ok(Step::Effect(Effect::Read)));
    let a_write = a.step(Input::Row(read(&store))).unwrap();
    let b_write = b.step(Input::Row(read(&store))).unwrap();

    // A commits first.
    let Step::Effect(Effect::Write {
        row,
        expected_version,
    }) = a_write
    else {
        panic!("{a_write:?}")
    };
    assert!(store.put_if_version(id, row.encode(id), row.version(), expected_version));
    assert_eq!(a.step(Input::WriteOk), Ok(Step::Done(1)));

    // B's write is rejected by the store.
    let Step::Effect(Effect::Write {
        row,
        expected_version,
    }) = b_write
    else {
        panic!("{b_write:?}")
    };
    assert_eq!(expected_version, 0);
    assert!(!store.put_if_version(id, row.encode(id), row.version(), expected_version));
    let sleep = b.step(Input::WriteConflict).unwrap();
    assert!(matches!(sleep, Step::Effect(Effect::Sleep(_))));

    // B re-reads, sees A's write, and commits on top of it.
    assert_eq!(b.resume(), Ok(Step::Effect(Effect::Read)));
    let retry = b.step(Input::Row(read(&store))).unwrap();
    let Step::Effect(Effect::Write {
        row,
        expected_version,
    }) = retry
    else {
        panic!("{retry:?}")
    };
    assert_eq!((expected_version, row.value), (1, 2));
    assert!(store.put_if_version(id, row.encode(id), row.version(), expected_version));
    assert_eq!(b.step(Input::WriteOk), Ok(Step::Done(2)));
    assert_eq!(b.attempts(), 2);

    assert_eq!(
        read(&store),
        Some(CounterRow {
            version: 2,
            value: 2
        })
    );
}
