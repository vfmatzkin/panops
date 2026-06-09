//! Library facade for `panops-engine` so integration tests can drive
//! the server in-process. The binary entry point lives in `main.rs`.
pub mod asr_resolver;
pub mod capture_resolver;
pub mod llm_resolver;
pub mod screenshots;
pub mod server;
mod sidecar_binary;
