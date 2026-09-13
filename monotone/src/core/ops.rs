//! A machine for each counter and queue operation.

use rand::rngs::SmallRng;

use super::error::Error;
use super::machine::{Machine, Modify, ReadOnly, RetryPolicy, Rmw};
use super::row::{CounterRow, Join, QueueRow};
use crate::{FencingToken, Tags, Ticket};

/// Reads the counter value. A missing row reads as 0.
pub fn get_value() -> impl Machine<Row = CounterRow, Output = u64> {
    ReadOnly::new(|row: Option<CounterRow>| Ok(row.map_or(0, |r| r.value)))
}

/// Increments the counter and returns the new value.
pub fn next_value(
    policy: RetryPolicy,
    rng: SmallRng,
) -> impl Machine<Row = CounterRow, Output = u64> {
    Rmw::new(
        |row: Option<CounterRow>| {
            let row = row.unwrap_or_default().increment();
            let value = row.value;
            Modify::Write(row, value)
        },
        policy,
        rng,
    )
}

/// Joins the queue, or returns the existing ticket without writing.
pub fn join_queue(
    process_id: &str,
    tags: Tags,
    policy: RetryPolicy,
    rng: SmallRng,
) -> impl Machine<Row = QueueRow, Output = (FencingToken, Ticket)> {
    let process_id = process_id.to_owned();
    Rmw::new(
        move |row: Option<QueueRow>| match row.unwrap_or_default().join(&process_id, tags.clone()) {
            Join::Existing(token, ticket) => Modify::Done((token, ticket)),
            Join::Added { row, ticket } => {
                let token = row.version;
                Modify::Write(row, (token, ticket))
            }
        },
        policy,
        rng,
    )
}

/// Leaves the queue and returns the new fencing token.
pub fn leave_queue(
    process_id: &str,
    policy: RetryPolicy,
    rng: SmallRng,
) -> impl Machine<Row = QueueRow, Output = FencingToken> {
    let process_id = process_id.to_owned();
    Rmw::new(
        move |row: Option<QueueRow>| match row.unwrap_or_default().leave(&process_id) {
            Ok(row) => {
                let token = row.version;
                Modify::Write(row, token)
            }
            Err(e) => Modify::Fail(e),
        },
        policy,
        rng,
    )
}

/// Looks up one process's ticket.
pub fn get_ticket(
    process_id: &str,
) -> impl Machine<Row = QueueRow, Output = (FencingToken, Ticket)> {
    let process_id = process_id.to_owned();
    ReadOnly::new(move |row: Option<QueueRow>| -> Result<_, Error> {
        row.unwrap_or_default().ticket(&process_id)
    })
}

/// Lists every ticket. A missing row lists token 0 and no tickets.
pub fn get_tickets() -> impl Machine<Row = QueueRow, Output = (FencingToken, Vec<Ticket>)> {
    ReadOnly::new(|row: Option<QueueRow>| Ok(row.unwrap_or_default().tickets()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rand::SeedableRng;

    use super::*;
    use crate::core::machine::script::run;
    use crate::core::machine::{Effect, Input, Step};
    use crate::core::row::QueueEntry;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(7)
    }

    fn counter(v: u64) -> CounterRow {
        CounterRow {
            version: v,
            value: v,
        }
    }

    fn queue_with(version: u64, entries: &[(&str, u64)]) -> QueueRow {
        QueueRow {
            version,
            value: entries.iter().map(|e| e.1).max().unwrap_or(0),
            items: entries
                .iter()
                .map(|(p, c)| QueueEntry {
                    process_id: p.to_string(),
                    counter: *c,
                    tags: Tags::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn read_only_emits_one_read_then_completes() {
        let r = run(
            &mut get_value(),
            vec![Input::Row(Some(CounterRow {
                version: 9,
                value: 7,
            }))],
        );
        assert_eq!(r.effects, vec![Effect::Read]);
        assert_eq!(r.result, Ok(7));
    }

    #[test]
    fn get_tickets_on_missing_row_is_empty_without_write() {
        let r = run(&mut get_tickets(), vec![Input::Row(None)]);
        assert_eq!(r.effects, vec![Effect::Read]);
        assert_eq!(r.result, Ok((0, vec![])));
    }

    #[test]
    fn next_value_without_contention() {
        let r = run(
            &mut next_value(RetryPolicy::default(), rng()),
            vec![Input::Row(Some(counter(3))), Input::WriteOk],
        );
        assert_eq!(
            r.effects,
            vec![
                Effect::Read,
                Effect::Write {
                    row: counter(4),
                    expected_version: 3
                }
            ]
        );
        assert_eq!(r.result, Ok(4));
    }

    #[test]
    fn first_write_to_missing_row_expects_version_zero() {
        let r = run(
            &mut next_value(RetryPolicy::default(), rng()),
            vec![Input::Row(None), Input::WriteOk],
        );
        assert_eq!(
            r.effects[1],
            Effect::Write {
                row: counter(1),
                expected_version: 0
            }
        );
        assert_eq!(r.result, Ok(1));
    }

    #[test]
    fn mismatched_input_is_a_protocol_error_and_finishes() {
        let mut m = get_value();
        assert_eq!(m.start(), Ok(Step::Effect(Effect::Read)));
        assert!(matches!(m.step(Input::WriteOk), Err(Error::Protocol(_))));
        assert!(matches!(m.step(Input::Row(None)), Err(Error::Protocol(_))));
        assert!(matches!(m.start(), Err(Error::Protocol(_))));

        let mut m = next_value(RetryPolicy::default(), rng());
        assert_eq!(m.start(), Ok(Step::Effect(Effect::Read)));
        assert!(matches!(
            m.step(Input::WriteConflict),
            Err(Error::Protocol(_))
        ));
        assert!(matches!(m.resume(), Err(Error::Protocol(_))));
    }

    #[test]
    fn rmw_rejects_out_of_order_calls() {
        let mut m = next_value(RetryPolicy::default(), rng());
        assert!(matches!(m.step(Input::Row(None)), Err(Error::Protocol(_))));

        let mut m = next_value(RetryPolicy::default(), rng());
        m.start().unwrap();
        assert!(matches!(m.resume(), Err(Error::Protocol(_))));

        let mut m = next_value(RetryPolicy::default(), rng());
        m.start().unwrap();
        m.step(Input::Row(None)).unwrap();
        assert!(matches!(m.step(Input::Row(None)), Err(Error::Protocol(_))));

        let mut m = next_value(RetryPolicy::default(), rng());
        m.start().unwrap();
        m.step(Input::Row(None)).unwrap();
        m.step(Input::WriteConflict).unwrap();
        assert!(matches!(m.step(Input::Row(None)), Err(Error::Protocol(_))));

        let mut m = next_value(RetryPolicy::default(), rng());
        m.start().unwrap();
        m.step(Input::Row(None)).unwrap();
        assert!(matches!(m.step(Input::WriteOk), Ok(Step::Done(1))));
        assert!(matches!(m.step(Input::WriteOk), Err(Error::Protocol(_))));
        assert!(matches!(m.start(), Err(Error::Protocol(_))));
    }

    #[test]
    fn modify_that_skips_a_version_is_rejected() {
        let mut m = Rmw::new(
            |_row: Option<CounterRow>| Modify::Write(counter(5), 5u64),
            RetryPolicy::default(),
            rng(),
        );
        m.start().unwrap();
        assert!(matches!(m.step(Input::Row(None)), Err(Error::Protocol(_))));
    }

    #[test]
    fn three_conflicts_then_success() {
        let mut m = next_value(RetryPolicy::default(), rng());
        let r = run(
            &mut m,
            vec![
                Input::Row(Some(counter(1))),
                Input::WriteConflict,
                Input::Row(Some(counter(2))),
                Input::WriteConflict,
                Input::Row(Some(counter(3))),
                Input::WriteConflict,
                Input::Row(Some(counter(4))),
                Input::WriteOk,
            ],
        );
        assert_eq!(r.count(|e| matches!(e, Effect::Sleep(_))), 3);
        assert_eq!(r.count(|e| matches!(e, Effect::Read)), 4);

        // Every write expects the version it had just read.
        let mut last_read = None;
        let mut feed = [1u64, 2, 3, 4].into_iter();
        for e in &r.effects {
            match e {
                Effect::Read => last_read = feed.next(),
                Effect::Write {
                    row,
                    expected_version,
                } => {
                    assert_eq!(Some(*expected_version), last_read);
                    assert_eq!(row.version, expected_version + 1);
                }
                Effect::Sleep(_) => {}
            }
        }
        assert_eq!(r.result, Ok(5));
        assert_eq!(m.attempts(), 4);
    }

    #[test]
    fn retry_observes_a_rejoin() {
        let r = run(
            &mut join_queue("foo", Tags::new(), RetryPolicy::default(), rng()),
            vec![
                Input::Row(None),
                Input::WriteConflict,
                Input::Row(Some(queue_with(1, &[("foo", 1)]))),
            ],
        );
        assert_eq!(r.count(|e| matches!(e, Effect::Write { .. })), 1);
        let (token, ticket) = r.result.unwrap();
        assert_eq!((token, ticket.counter, ticket.position), (1, 1, 0));
    }

    #[test]
    fn retry_observes_a_removal() {
        let r = run(
            &mut leave_queue("foo", RetryPolicy::default(), rng()),
            vec![
                Input::Row(Some(queue_with(1, &[("foo", 1)]))),
                Input::WriteConflict,
                Input::Row(Some(queue_with(2, &[]))),
            ],
        );
        assert_eq!(r.count(|e| matches!(e, Effect::Write { .. })), 1);
        assert_eq!(r.result, Err(Error::NotFound("foo".into())));
    }

    #[test]
    fn leave_on_missing_row_fails_immediately() {
        let r = run(
            &mut leave_queue("foo", RetryPolicy::default(), rng()),
            vec![Input::Row(None)],
        );
        assert_eq!(r.effects, vec![Effect::Read]);
        assert_eq!(r.result, Err(Error::NotFound("foo".into())));
    }

    #[test]
    fn get_ticket_on_missing_row_is_not_found() {
        let r = run(&mut get_ticket("foo"), vec![Input::Row(None)]);
        assert_eq!(r.result, Err(Error::NotFound("foo".into())));
    }

    fn sleeps(seed: u64) -> Vec<Duration> {
        let policy = RetryPolicy {
            retry_time: Duration::from_millis(100),
            jitter_millis: 50,
        };
        let r = run(
            &mut next_value(policy, SmallRng::seed_from_u64(seed)),
            vec![
                Input::Row(None),
                Input::WriteConflict,
                Input::Row(None),
                Input::WriteConflict,
                Input::Row(None),
                Input::WriteOk,
            ],
        );
        r.effects
            .into_iter()
            .filter_map(|e| match e {
                Effect::Sleep(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn same_seed_gives_same_sleeps_within_bounds() {
        let a = sleeps(42);
        assert_eq!(a, sleeps(42));
        assert_eq!(a.len(), 2);
        for d in a {
            assert!(
                d >= Duration::from_millis(100) && d <= Duration::from_millis(149),
                "{d:?}"
            );
        }
    }

    #[test]
    fn zero_jitter_sleeps_exactly_the_retry_time() {
        let policy = RetryPolicy {
            retry_time: Duration::from_millis(5),
            jitter_millis: 0,
        };
        let r = run(
            &mut next_value(policy, rng()),
            vec![
                Input::Row(None),
                Input::WriteConflict,
                Input::Row(None),
                Input::WriteOk,
            ],
        );
        assert!(r.effects.contains(&Effect::Sleep(Duration::from_millis(5))));
    }

    #[test]
    fn default_policy_has_100ms_jitter() {
        assert_eq!(RetryPolicy::default().jitter_millis, 100);
    }
}
