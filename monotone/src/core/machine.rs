//! The effect protocol and the machines that implement it.
//!
//! A [`Machine`] performs no I/O. It asks for work through [`Effect`]s and
//! learns the outcome through [`Input`]s:
//!
//! | Pending effect | How the backend answers |
//! |----------------|-------------------------|
//! | [`Effect::Read`] | [`Machine::step`] with [`Input::Row`] |
//! | [`Effect::Write`] | [`Machine::step`] with [`Input::WriteOk`] or [`Input::WriteConflict`] |
//! | [`Effect::Sleep`] | wait, then [`Machine::resume`] |
//!
//! A `Write` must succeed only if the stored row's version equals
//! `expected_version`, or no row exists and `expected_version` is 0.
//! Any mismatch between the pending effect and the answer is a
//! [`Error::Protocol`] and finishes the machine.

use std::time::Duration;

use rand::RngExt;
use rand::rngs::SmallRng;

use super::error::Error;
use super::row::Row;

/// Work a machine asks its backend to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect<R> {
    /// Fetch the row, using a strongly consistent read.
    Read,
    /// Store `row` only if the stored version equals `expected_version`
    /// (or no row exists and `expected_version` is 0).
    Write {
        /// The complete row to store. Its version is `expected_version + 1`.
        row: R,
        /// The version the row must currently have.
        expected_version: u64,
    },
    /// Wait for the duration, then call [`Machine::resume`].
    Sleep(Duration),
}

/// A backend's answer to a pending [`Effect::Read`] or [`Effect::Write`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input<R> {
    /// The row that was read, or `None` if no row exists.
    Row(Option<R>),
    /// The conditional write succeeded.
    WriteOk,
    /// The conditional write was rejected because the version had changed.
    WriteConflict,
}

/// What a machine wants next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<R, T> {
    /// Perform this effect and report back.
    Effect(Effect<R>),
    /// The operation finished with this output.
    Done(T),
}

/// Retry timing after a write conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Base delay before re-reading.
    pub retry_time: Duration,
    /// Exclusive upper bound, in milliseconds, of the uniform random jitter
    /// added to `retry_time`. 0 disables jitter.
    pub jitter_millis: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            retry_time: Duration::from_millis(100),
            jitter_millis: 100,
        }
    }
}

/// An operation expressed as an effect-driven state machine.
pub trait Machine {
    /// The row type the machine reads and writes.
    type Row: Row;
    /// The operation's result.
    type Output;

    /// Begins the operation. Returns the first effect.
    fn start(&mut self) -> Result<Step<Self::Row, Self::Output>, Error>;

    /// Answers the pending `Read` or `Write`.
    fn step(&mut self, input: Input<Self::Row>) -> Result<Step<Self::Row, Self::Output>, Error>;

    /// Continues after a pending `Sleep`.
    fn resume(&mut self) -> Result<Step<Self::Row, Self::Output>, Error>;

    /// Number of writes emitted so far. Read-only machines never write.
    fn attempts(&self) -> u32 {
        0
    }
}

/// Outcome of a read-modify-write `modify` function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modify<R, T> {
    /// Write `R`; if the write succeeds the operation returns `T`.
    Write(R, T),
    /// Finish with `T` without writing.
    Done(T),
    /// Finish with an error without writing.
    Fail(Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadState {
    Ready,
    AwaitingRow,
    Finished,
}

/// A machine that reads once and projects the row into a result.
pub struct ReadOnly<R, F> {
    state: ReadState,
    project: Option<F>,
    _row: std::marker::PhantomData<fn() -> R>,
}

impl<R, T, F> ReadOnly<R, F>
where
    R: Row,
    F: FnOnce(Option<R>) -> Result<T, Error>,
{
    /// Builds a read-only machine from a projection of the (possibly missing) row.
    pub fn new(project: F) -> Self {
        ReadOnly {
            state: ReadState::Ready,
            project: Some(project),
            _row: std::marker::PhantomData,
        }
    }

    fn fail(&mut self, reason: &'static str) -> Result<Step<R, T>, Error> {
        self.state = ReadState::Finished;
        Err(Error::Protocol(reason))
    }
}

impl<R, T, F> Machine for ReadOnly<R, F>
where
    R: Row,
    F: FnOnce(Option<R>) -> Result<T, Error>,
{
    type Row = R;
    type Output = T;

    fn start(&mut self) -> Result<Step<R, T>, Error> {
        match self.state {
            ReadState::Ready => {
                self.state = ReadState::AwaitingRow;
                Ok(Step::Effect(Effect::Read))
            }
            ReadState::AwaitingRow => self.fail("start called twice"),
            ReadState::Finished => self.fail("machine already finished"),
        }
    }

    fn step(&mut self, input: Input<R>) -> Result<Step<R, T>, Error> {
        match (self.state, input) {
            (ReadState::AwaitingRow, Input::Row(row)) => {
                self.state = ReadState::Finished;
                let project = self
                    .project
                    .take()
                    .expect("projection present until finished");
                project(row).map(Step::Done)
            }
            (ReadState::AwaitingRow, _) => self.fail("expected a row in reply to Read"),
            (ReadState::Ready, _) => self.fail("step called before start"),
            (ReadState::Finished, _) => self.fail("machine already finished"),
        }
    }

    fn resume(&mut self) -> Result<Step<R, T>, Error> {
        self.fail("resume called with no pending Sleep")
    }
}

enum RmwState<T> {
    Ready,
    AwaitingRow,
    AwaitingWrite(T),
    Sleeping,
    Finished,
}

/// The read-modify-write machine behind every mutating operation.
///
/// It reads the row, applies `modify`, and writes the result conditionally on
/// the version it read. On a conflict it sleeps for the retry time plus
/// jitter, re-reads, and recomputes from the fresh row, without limit.
pub struct Rmw<R, T, F> {
    state: RmwState<T>,
    modify: F,
    policy: RetryPolicy,
    rng: SmallRng,
    attempts: u32,
    _row: std::marker::PhantomData<fn() -> R>,
}

impl<R, T, F> Rmw<R, T, F>
where
    R: Row,
    F: FnMut(Option<R>) -> Modify<R, T>,
{
    /// Builds a machine. `rng` supplies jitter; seed it for reproducible sleeps.
    pub fn new(modify: F, policy: RetryPolicy, rng: SmallRng) -> Self {
        Rmw {
            state: RmwState::Ready,
            modify,
            policy,
            rng,
            attempts: 0,
            _row: std::marker::PhantomData,
        }
    }

    fn fail(&mut self, reason: &'static str) -> Result<Step<R, T>, Error> {
        self.state = RmwState::Finished;
        Err(Error::Protocol(reason))
    }

    fn backoff(&mut self) -> Duration {
        let jitter = match self.policy.jitter_millis {
            0 => 0,
            bound => self.rng.random_range(0..bound),
        };
        self.policy.retry_time + Duration::from_millis(jitter)
    }
}

impl<R, T, F> Machine for Rmw<R, T, F>
where
    R: Row,
    F: FnMut(Option<R>) -> Modify<R, T>,
{
    type Row = R;
    type Output = T;

    fn start(&mut self) -> Result<Step<R, T>, Error> {
        match self.state {
            RmwState::Ready => {
                self.state = RmwState::AwaitingRow;
                Ok(Step::Effect(Effect::Read))
            }
            RmwState::Finished => self.fail("machine already finished"),
            _ => self.fail("start called twice"),
        }
    }

    fn step(&mut self, input: Input<R>) -> Result<Step<R, T>, Error> {
        let state = std::mem::replace(&mut self.state, RmwState::Finished);
        match (state, input) {
            (RmwState::AwaitingRow, Input::Row(row)) => {
                let expected_version = row.as_ref().map_or(0, Row::version);
                match (self.modify)(row) {
                    Modify::Write(row, output) => {
                        if row.version() != expected_version + 1 {
                            return self.fail("modify must bump the version by exactly one");
                        }
                        self.attempts += 1;
                        self.state = RmwState::AwaitingWrite(output);
                        Ok(Step::Effect(Effect::Write {
                            row,
                            expected_version,
                        }))
                    }
                    Modify::Done(output) => Ok(Step::Done(output)),
                    Modify::Fail(error) => Err(error),
                }
            }
            (RmwState::AwaitingWrite(output), Input::WriteOk) => Ok(Step::Done(output)),
            (RmwState::AwaitingWrite(_), Input::WriteConflict) => {
                self.state = RmwState::Sleeping;
                Ok(Step::Effect(Effect::Sleep(self.backoff())))
            }
            (RmwState::AwaitingRow, _) => self.fail("expected a row in reply to Read"),
            (RmwState::AwaitingWrite(_), _) => {
                self.fail("expected a write result in reply to Write")
            }
            (RmwState::Sleeping, _) => self.fail("expected resume after Sleep"),
            (RmwState::Ready, _) => self.fail("step called before start"),
            (RmwState::Finished, _) => self.fail("machine already finished"),
        }
    }

    fn resume(&mut self) -> Result<Step<R, T>, Error> {
        match self.state {
            RmwState::Sleeping => {
                self.state = RmwState::AwaitingRow;
                Ok(Step::Effect(Effect::Read))
            }
            _ => self.fail("resume called with no pending Sleep"),
        }
    }

    fn attempts(&self) -> u32 {
        self.attempts
    }
}

/// Test support: runs a machine against a script of inputs, recording effects.
#[cfg(test)]
pub(crate) mod script {
    use super::*;

    /// Record of a scripted run.
    #[derive(Debug)]
    pub struct Run<R, T> {
        pub effects: Vec<Effect<R>>,
        pub result: Result<T, Error>,
    }

    impl<R, T> Run<R, T> {
        pub fn count(&self, pred: impl Fn(&Effect<R>) -> bool) -> usize {
            self.effects.iter().filter(|e| pred(e)).count()
        }
    }

    /// Feeds `inputs` in order, answering each `Sleep` with `resume`.
    /// Panics if the machine asks for more inputs than the script holds.
    pub fn run<M: Machine>(m: &mut M, inputs: Vec<Input<M::Row>>) -> Run<M::Row, M::Output>
    where
        M::Row: Clone,
    {
        let mut effects = Vec::new();
        let mut inputs = inputs.into_iter();
        let mut step = m.start();
        loop {
            match step {
                Err(e) => {
                    return Run {
                        effects,
                        result: Err(e),
                    };
                }
                Ok(Step::Done(t)) => {
                    return Run {
                        effects,
                        result: Ok(t),
                    };
                }
                Ok(Step::Effect(effect)) => {
                    let sleeping = matches!(effect, Effect::Sleep(_));
                    effects.push(effect);
                    step = if sleeping {
                        m.resume()
                    } else {
                        m.step(inputs.next().expect("script ran out of inputs"))
                    };
                }
            }
        }
    }
}
