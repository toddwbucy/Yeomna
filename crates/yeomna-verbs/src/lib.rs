//! The verb contract: Yeomna's closed vocabulary as types.
//!
//! Built per `docs/specs/010-verb-contract/spec.md`, executing the
//! verb-layer PRD's Phase 1. This crate is types only: no I/O, no SQL, no
//! store dependency. The verb implementations arrive in later phases and
//! this crate is the shape they all speak, the R4 name close made binding,
//! and the wire contract the daemon will frame.
//!
//! Three properties the types enforce:
//!
//! - **The vocabulary is closed.** [`Verb`] is one exhaustive enum. An
//!   unknown wire name is a deserialization error naming the stranger, not
//!   an extension point.
//! - **Requests are strict.** Every request struct denies unknown fields,
//!   so a client cannot smuggle an argument (an `actor`, most importantly)
//!   past the contract. The actor is the kernel's answer at the socket,
//!   never a request field (PRD V3).
//! - **Presentation stays out.** The captured CLI's `format` and `verbose`
//!   flags are rendering concerns and do not exist here.

mod audit;
mod database;
mod envelope;
mod error;
mod execute;
mod graph;
mod read;
mod verb;

pub use envelope::{Envelope, envelope, error_envelope};
pub use error::VerbError;
pub use execute::Session;
/// The traversal SQL, exposed so the pruning test EXPLAINs exactly what
/// the verb executes rather than a copy that drifts (spec 013 FR 1).
pub use graph::traverse_sql;
pub use verb::*;
