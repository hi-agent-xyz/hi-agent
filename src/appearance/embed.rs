//! Compile-time embed of the built SPA.
//!
//! The folder path is relative to the crate root, and must exist when this derive
//! expands or the crate does not compile — `build.rs` creates it for that reason.
//! During development it may be empty, in which case `WebAssets::get` returns
//! `None` for everything and our handlers fall through to the dev placeholder.
//! In release builds, `npm run build` populates `dist/` before `cargo build`.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "src/appearance/web/dist/"]
pub struct WebAssets;

/// Convenience wrapper around `WebAssets::get` so callers don't import the
/// `RustEmbed` trait themselves.
pub fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
    WebAssets::get(path)
}

/// The type an embedded asset is served as — [the one table](crate::mind::memory::media::content_type),
/// which covers what Vite produces because this route's needs are in it rather than beside it.
pub fn content_type_for(path: &str) -> &'static str {
    crate::mind::memory::media::content_type(path)
}
