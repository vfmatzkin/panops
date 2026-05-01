//! Test-time helpers shared between adapter test crates.
//!
//! Adapters call `conformance::asr::run_suite` or `conformance::diar::run_suite`
//! from their own integration tests.

pub mod asr;
pub mod diar;
pub mod fakes;
pub mod llm;
