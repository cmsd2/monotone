//! Property tests over the pure core.
//!
//! The harness keeps one counter slot and one queue slot. Each generated
//! operation runs through its machine. Optionally, another operation commits
//! between the machine's first read and its write, which forces a genuine
//! version conflict. Invariants are checked after every step, and the final
//! state is compared with a sequential model that applies the same commits in
//! order.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use super::encode::Codec;
use super::error::Error;
use super::machine::{Effect, Input, Machine, RetryPolicy, Step};
use super::ops;
use super::row::{CounterRow, Join, QueueEntry, QueueRow, Row};
use crate::{Tags, Ticket};

#[derive(Debug, Clone)]
enum Op {
    Next,
    Join(u8, Tags),
    Leave(u8),
    Ticket(u8),
    Tickets,
    Value,
}

fn pid(p: u8) -> String {
    format!("p{p}")
}

fn tags() -> impl Strategy<Value = Tags> {
    prop::collection::btree_map("[a-z]{1,3}", "[a-z0-9=]{0,4}", 0..3)
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Next),
        4 => (0u8..6, tags()).prop_map(|(p, t)| Op::Join(p, t)),
        3 => (0u8..6).prop_map(Op::Leave),
        1 => (0u8..6).prop_map(Op::Ticket),
        1 => Just(Op::Tickets),
        1 => Just(Op::Value),
    ]
}

fn policy() -> RetryPolicy {
    RetryPolicy {
        retry_time: std::time::Duration::ZERO,
        jitter_millis: 3,
    }
}

/// Output, rows read, and whether a write committed.
type Driven<M> = (
    Result<<M as Machine>::Output, Error>,
    Vec<Option<<M as Machine>::Row>>,
    bool,
);

/// Drives `m` against `slot`. `interfere` runs once, after the first read.
/// Returns the output, the rows read, and whether a write committed.
fn drive<M: Machine>(
    m: &mut M,
    slot: &mut Option<M::Row>,
    mut interfere: impl FnMut(&mut Option<M::Row>),
) -> Driven<M> {
    let mut reads = Vec::new();
    let mut committed = false;
    let mut step = m.start();
    loop {
        step = match step {
            Err(e) => return (Err(e), reads, committed),
            Ok(Step::Done(out)) => return (Ok(out), reads, committed),
            Ok(Step::Effect(Effect::Read)) => {
                let row = slot.clone();
                reads.push(row.clone());
                let next = m.step(Input::Row(row));
                if reads.len() == 1 {
                    interfere(slot);
                }
                next
            }
            Ok(Step::Effect(Effect::Write {
                row,
                expected_version,
            })) => {
                let current = slot.as_ref().map_or(0, Row::version);
                if current == expected_version {
                    *slot = Some(row);
                    committed = true;
                    m.step(Input::WriteOk)
                } else {
                    m.step(Input::WriteConflict)
                }
            }
            Ok(Step::Effect(Effect::Sleep(_))) => m.resume(),
        };
    }
}

#[derive(Default)]
struct World {
    counter: Option<CounterRow>,
    queue: Option<QueueRow>,
    counter_writes: u64,
    queue_writes: u64,
    /// Every (counter -> process) ever issued.
    issued: BTreeMap<u64, String>,
}

/// Applies an op with no interference, as a concurrent writer would.
fn commit(world: &mut World, op: &Op) {
    let seed = 99;
    match op {
        Op::Next => {
            let (_, _, c) = drive(
                &mut ops::next_value(policy(), SmallRng::seed_from_u64(seed)),
                &mut world.counter,
                |_| {},
            );
            world.counter_writes += c as u64;
        }
        Op::Join(p, t) => {
            let (_, _, c) = drive(
                &mut ops::join_queue(&pid(*p), t.clone(), policy(), SmallRng::seed_from_u64(seed)),
                &mut world.queue,
                |_| {},
            );
            world.queue_writes += c as u64;
        }
        Op::Leave(p) => {
            let (_, _, c) = drive(
                &mut ops::leave_queue(&pid(*p), policy(), SmallRng::seed_from_u64(seed)),
                &mut world.queue,
                |_| {},
            );
            world.queue_writes += c as u64;
        }
        Op::Ticket(_) | Op::Tickets | Op::Value => {}
    }
}

fn check_queue_invariants(world: &mut World) {
    let row = world.queue.clone().unwrap_or_default();
    let (token, tickets) = row.tickets();
    assert_eq!(token, world.queue_writes, "version bumps once per write");

    let positions: Vec<usize> = tickets.iter().map(|t| t.position).collect();
    assert_eq!(
        positions,
        (0..tickets.len()).collect::<Vec<_>>(),
        "positions dense"
    );
    assert!(
        tickets.windows(2).all(|w| w[0].counter < w[1].counter),
        "counters ascending"
    );
    let pids: BTreeSet<_> = tickets.iter().map(|t| &t.process_id).collect();
    assert_eq!(pids.len(), tickets.len(), "process ids unique");
    assert!(
        tickets.iter().all(|t| t.counter <= row.value),
        "value is the highest issued"
    );

    for t in &tickets {
        match world.issued.get(&t.counter) {
            Some(owner) => assert_eq!(owner, &t.process_id, "counter {} reused", t.counter),
            None => {
                world.issued.insert(t.counter, t.process_id.clone());
            }
        }
    }
}

fn is_counter_op(op: &Op) -> bool {
    matches!(op, Op::Next | Op::Value)
}

fn run_op(world: &mut World, op: &Op, interference: Option<&Op>) {
    // Interference on the other slot cannot conflict with this op, so commit it
    // up front. Same-slot interference commits between read and write.
    let interference = match interference {
        Some(side) if is_counter_op(side) != is_counter_op(op) => {
            commit(world, side);
            if !is_counter_op(side) {
                check_queue_invariants(world);
            }
            None
        }
        other => other,
    };
    let rng = SmallRng::seed_from_u64(1);
    match op {
        Op::Next | Op::Value => {
            let mut w = std::mem::take(world);
            let mut slot = w.counter.take();
            let mut side = interference.cloned();
            let mut interfere = |s: &mut Option<CounterRow>| {
                if let Some(o) = side.take() {
                    let mut inner = World {
                        counter: s.take(),
                        counter_writes: w.counter_writes,
                        ..World::default()
                    };
                    commit(&mut inner, &o);
                    *s = inner.counter;
                    w.counter_writes = inner.counter_writes;
                }
            };
            if matches!(op, Op::Next) {
                let (out, reads, committed) = drive(
                    &mut ops::next_value(policy(), rng),
                    &mut slot,
                    &mut interfere,
                );
                let value = out.expect("next_value never fails");
                let last = reads.last().unwrap().clone().unwrap_or_default();
                assert_eq!(value, last.value + 1, "result matches last read");
                assert!(committed);
                assert_eq!(slot.as_ref().unwrap().value, value);
                w.counter_writes += 1;
            } else {
                let (out, reads, _) = drive(&mut ops::get_value(), &mut slot, &mut interfere);
                assert_eq!(out.unwrap(), reads[0].as_ref().map_or(0, |r| r.value));
            }
            w.counter = slot;
            assert_eq!(
                w.counter.as_ref().map_or(0, |r| r.version),
                w.counter_writes
            );
            *world = w;
        }
        Op::Join(..) | Op::Leave(_) | Op::Ticket(_) | Op::Tickets => {
            let mut w = std::mem::take(world);
            let mut slot = w.queue.take();
            let mut side = interference.cloned();
            let mut interfere = |s: &mut Option<QueueRow>| {
                if let Some(o) = side.take() {
                    let mut inner = World {
                        queue: s.take(),
                        queue_writes: w.queue_writes,
                        ..World::default()
                    };
                    commit(&mut inner, &o);
                    *s = inner.queue;
                    w.queue_writes = inner.queue_writes;
                }
            };
            match op {
                Op::Join(p, t) => {
                    let (out, reads, committed) = drive(
                        &mut ops::join_queue(&pid(*p), t.clone(), policy(), rng),
                        &mut slot,
                        &mut interfere,
                    );
                    let (token, ticket) = out.expect("join never fails");
                    let last = reads.last().unwrap().clone().unwrap_or_default();
                    if committed {
                        let now = slot.clone().unwrap();
                        assert_eq!(now.ticket(&pid(*p)), Ok((now.version, ticket.clone())));
                        assert_eq!(token, last.version + 1);
                        assert_eq!(ticket.counter, last.value + 1);
                        assert_eq!(&ticket.tags, t);
                        w.queue_writes += 1;
                    } else {
                        assert_eq!(Ok((token, ticket)), last.ticket(&pid(*p)));
                    }
                }
                Op::Leave(p) => {
                    let (out, reads, committed) = drive(
                        &mut ops::leave_queue(&pid(*p), policy(), rng),
                        &mut slot,
                        &mut interfere,
                    );
                    let last = reads.last().unwrap().clone().unwrap_or_default();
                    match out {
                        Ok(token) => {
                            assert!(committed);
                            assert_eq!(token, last.version + 1);
                            assert_eq!(slot.as_ref().unwrap().version, token);
                            w.queue_writes += 1;
                        }
                        Err(e) => {
                            assert!(!committed);
                            assert_eq!(e, Error::NotFound(pid(*p)));
                            assert!(last.ticket(&pid(*p)).is_err());
                        }
                    }
                }
                Op::Ticket(p) => {
                    let (out, reads, _) =
                        drive(&mut ops::get_ticket(&pid(*p)), &mut slot, &mut interfere);
                    assert_eq!(out, reads[0].clone().unwrap_or_default().ticket(&pid(*p)));
                }
                Op::Tickets => {
                    let (out, reads, _) = drive(&mut ops::get_tickets(), &mut slot, &mut interfere);
                    assert_eq!(out.unwrap(), reads[0].clone().unwrap_or_default().tickets());
                }
                _ => unreachable!(),
            }
            w.queue = slot;
            *world = w;
            check_queue_invariants(world);
        }
    }
}

/// Sequential model using the pure row functions directly.
fn model(ops: &[(Op, Option<Op>)]) -> (CounterRow, QueueRow) {
    let mut c = CounterRow::default();
    let mut q = QueueRow::default();
    let apply = |op: &Op, c: &mut CounterRow, q: &mut QueueRow| match op {
        Op::Next => *c = c.increment(),
        Op::Join(p, t) => {
            if let Join::Added { row, .. } = q.join(&pid(*p), t.clone()) {
                *q = row;
            }
        }
        Op::Leave(p) => {
            if let Ok(row) = q.leave(&pid(*p)) {
                *q = row;
            }
        }
        _ => {}
    };
    // An op that writes linearizes after same-slot interference, because the
    // conflict forces it to re-read. An op that finishes on its first read
    // (a lookup, a rejoin, a failed leave) linearizes before it.
    let writes = |op: &Op, q: &QueueRow| match op {
        Op::Next => true,
        Op::Join(p, _) => q.ticket(&pid(*p)).is_err(),
        Op::Leave(p) => q.ticket(&pid(*p)).is_ok(),
        _ => false,
    };
    for (op, side) in ops {
        match side {
            Some(side) if writes(op, &q) => {
                apply(side, &mut c, &mut q);
                apply(op, &mut c, &mut q);
            }
            Some(side) => {
                apply(op, &mut c, &mut q);
                apply(side, &mut c, &mut q);
            }
            None => apply(op, &mut c, &mut q),
        }
    }
    (c, q)
}

fn counter_row() -> impl Strategy<Value = CounterRow> {
    (any::<u64>(), any::<u64>()).prop_map(|(version, value)| CounterRow { version, value })
}

fn queue_row() -> impl Strategy<Value = QueueRow> {
    (
        any::<u64>(),
        prop::collection::btree_map("[a-zA-Z0-9_.:-]{1,12}", (1u64..1000, tags()), 0..8),
        0u64..1000,
    )
        .prop_map(|(version, entries, headroom)| {
            let mut counter = 0;
            let items: Vec<QueueEntry> = entries
                .into_iter()
                .map(|(process_id, (gap, tags))| {
                    counter += gap;
                    QueueEntry {
                        process_id,
                        counter,
                        tags,
                    }
                })
                .collect();
            QueueRow {
                version,
                value: counter + headroom,
                items,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn invariants_hold_under_interleaved_conflicts(
        script in prop::collection::vec((op(), prop::option::weighted(0.4, op())), 1..40)
    ) {
        let mut world = World::default();
        for (op, side) in &script {
            run_op(&mut world, op, side.as_ref());
        }
        let (c, q) = model(&script);
        prop_assert_eq!(world.counter.unwrap_or_default(), c);
        prop_assert_eq!(world.queue.unwrap_or_default(), q);
    }

    #[test]
    fn counter_row_round_trips(row in counter_row(), id in "[a-z0-9-]{1,20}") {
        prop_assert_eq!(CounterRow::decode(&row.encode(&id)), Ok(row));
    }

    #[test]
    fn queue_row_round_trips(row in queue_row(), id in "[a-z0-9-]{1,20}") {
        prop_assert_eq!(QueueRow::decode(&row.encode(&id)), Ok(row));
    }
}

#[test]
fn same_slot_interference_forces_a_retry() {
    let mut slot: Option<CounterRow> = None;
    let mut m = ops::next_value(policy(), SmallRng::seed_from_u64(3));
    let mut fired = false;
    let (out, reads, committed) = drive(&mut m, &mut slot, |s| {
        fired = true;
        *s = Some(CounterRow::default().increment());
    });
    assert!(fired && committed);
    assert_eq!(reads.len(), 2, "conflict must cause a re-read");
    assert_eq!(m.attempts(), 2);
    assert_eq!(out, Ok(2));
}

#[test]
fn rejoin_after_interleaved_join_returns_the_winners_ticket() {
    let mut slot: Option<QueueRow> = None;
    let mut m = ops::join_queue("a", Tags::new(), policy(), SmallRng::seed_from_u64(3));
    let (out, reads, committed) = drive(&mut m, &mut slot, |s| {
        if let Join::Added { row, .. } = s.clone().unwrap_or_default().join("a", Tags::new()) {
            *s = Some(row);
        }
    });
    assert!(!committed);
    assert_eq!(reads.len(), 2);
    let expected: (u64, Ticket) = (
        1,
        Ticket {
            process_id: "a".into(),
            counter: 1,
            position: 0,
            tags: Tags::new(),
        },
    );
    assert_eq!(out, Ok(expected));
}
