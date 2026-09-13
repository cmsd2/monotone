//! The sans-IO core shared by every backend.
//!
//! - [`row`] holds the counter and queue row types and their pure operations.
//! - [`encode`] converts rows to and from a backend-neutral [`encode::Item`].
//! - [`machine`] defines the effect protocol and the read-modify-write driver.
//! - [`ops`] builds a machine for each counter and queue operation.
//!
//! A backend drives an operation like this:
//!
//! ```text
//! let mut step = machine.start()?;
//! loop {
//!     step = match step {
//!         Step::Done(output) => return Ok(output),
//!         Step::Effect(Effect::Read) => machine.step(Input::Row(read()?))?,
//!         Step::Effect(Effect::Write { row, expected_version }) => match put(row, expected_version) {
//!             Ok(()) => machine.step(Input::WriteOk)?,
//!             Err(Conflict) => machine.step(Input::WriteConflict)?,
//!         },
//!         Step::Effect(Effect::Sleep(d)) => { sleep(d); machine.resume()? }
//!     }
//! }
//! ```

pub mod encode;
pub mod error;
pub mod machine;
pub mod ops;
#[cfg(test)]
mod props;
pub mod row;
