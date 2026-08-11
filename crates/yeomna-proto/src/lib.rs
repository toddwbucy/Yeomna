//! Yeomna gRPC/protobuf definitions.
//!
//! Generated from `.proto` files under the workspace `proto/` directory.
//! Provides both client stubs and server traits.
//!
//! Lifted from HADES-Burn `crates/hades-proto` per
//! `docs/specs/003-proto-and-embed/spec.md`, trimmed to the one package with
//! a consumer in lift scope. The reference's `persephone.embedding` (dead
//! wire protocol of the deleted gRPC embedder), `persephone.common`
//! (imported by nothing), and `hades.training` (consumers outside lift
//! scope) stay behind and can be added the day a consumer arrives. The wire
//! package was renamed from `persephone.extraction` to `yeomna.extraction`
//! per the 2026-08-11 naming ruling in CLAUDE.md: nothing deployed speaks
//! the old name, and the mythology stays behind.

/// Extraction service — document content extraction.
pub mod extraction {
    tonic::include_proto!("yeomna.extraction");
}
