//! Test-time helpers shared between adapter test crates.
//!
//! Adapters call `conformance::asr::run_suite(&adapter, fixtures_dir)`
//! from their own integration tests. Pub-but-test-shaped: the harness is
//! a tool for verifying any `AsrProvider` implementation, not a
//! production API.

pub mod asr;
pub mod fakes;
