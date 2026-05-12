//! macOS-native adapters for panops. All public surface is gated on
//! `cfg(target_os = "macos")`; on other targets this crate is empty
//! and the engine resolver falls back to portable adapters.
//!
//! Slice 10 design:
//! `docs/superpowers/specs/2026-05-12-slice-10-whisperkit-asr-sidecar-design.md`.

#![cfg(target_os = "macos")]

mod whisperkit_asr;

pub use whisperkit_asr::WhisperKitAsr;
