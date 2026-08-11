//! Model service clients.
//!
//! Typed clients for the embedding and extraction compute services.
//! Transports differ by service:
//!
//! - **embedding** — OpenAI-compatible HTTP (PE-API v1 shape). Do **not**
//!   reintroduce the reference's pre-PR-#70 gRPC Unix-socket pattern for the
//!   embedder; that path was removed deliberately.
//! - **extraction** — gRPC over a Unix domain socket or TCP.
//!
//! Lifted from HADES-Burn `crates/hades-core/src/persephone/` per
//! `docs/specs/003-proto-and-embed/spec.md`. The move edits are `mod.rs`
//! becoming this `lib.rs`, `hades_proto` imports becoming `yeomna_proto`,
//! and this provenance note. Behavior is preserved by the change being a
//! move (PRD-pipeline-libraries G3). The reference's Persephone brand stays
//! behind per the 2026-08-11 naming ruling in CLAUDE.md.

pub mod embedding;
pub mod extraction;
