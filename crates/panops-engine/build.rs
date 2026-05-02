//! Add `LC_RPATH` entries to the macOS binary so it can locate its
//! sister dylibs (`libonnxruntime.*.dylib`, `libsherpa-onnx-c-api.dylib`)
//! at runtime without requiring `DYLD_LIBRARY_PATH` to be set.
//!
//! Today (issue #34) the binary references those dylibs via `@rpath/...`
//! (set by `sherpa-rs-sys`'s build) but has no rpath search list, so
//! distributing the binary fails at startup with
//! `Library not loaded: @rpath/libonnxruntime.1.17.1.dylib`.
//!
//! Strategy: emit two rpath search entries.
//!   1. `@executable_path` — covers `cargo run`, `cargo install`, and a
//!      flat layout where dylibs sit next to the binary.
//!   2. `@executable_path/../lib` — covers app-bundle layouts (`bin/` +
//!      `lib/`) which we'll use in slice 06 packaging.
//!
//! The script is gated on `cfg(target_os = "macos")` so Linux/Windows
//! builds are unaffected. `cargo install`'s known limitation: this rpath
//! does not bundle the dylibs themselves, so a user who `cargo install`s
//! the engine still needs the dylibs on their `DYLD_FALLBACK_LIBRARY_PATH`
//! or copied next to the installed binary. Slice 06 packaging will fix
//! that end-to-end.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../lib");
    }
}
