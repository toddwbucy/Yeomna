//! Model service clients.
//!
//! Typed clients for the embedding and extraction compute services.
//! Transports differ by service, and both are documented rulings rather
//! than accidents:
//!
//! - **embedding** - HTTP/1.1 with JSON bodies, `yeomna.embedding` v1 at
//!   `docs/embedding-contract.md`, over a Unix domain socket and nothing
//!   else. Do **not** reintroduce the reference's pre-PR-#70 gRPC
//!   Unix-socket pattern for the embedder, that path was removed
//!   deliberately and spec 022 kept the removal. Do **not** add a network
//!   transport: charter 5.1 makes local embedding a requirement rather
//!   than a configuration default, and spec 022 deleted the TCP variant
//!   so the type cannot express one (PRD-embedder D10).
//! - **extraction** - gRPC over a Unix domain socket or TCP.
//!
//! Two transports in one crate is a cost. It is smaller than reversing a
//! ruling to buy symmetry, and PRD-embedder D2 records the arithmetic.
//!
//! The embedding client was lifted from HADES-Burn
//! `crates/hades-core/src/persephone/` per
//! `docs/specs/003-proto-and-embed/spec.md` and rewritten in place by
//! spec 022, which forked the response shape: a response is always
//! chunked and the service pools, so nothing of PE-API's OpenAI kinship
//! survives except its judgement that there is no single-vector mode.
//! The extraction client is still the move. The reference's Persephone
//! brand stays behind per the 2026-08-11 naming ruling in CLAUDE.md.

pub mod embedding;
pub mod extraction;
