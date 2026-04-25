//! Encoder-side tuning knobs shared across every `*_file` pipeline:
//! JPEG quality, PNG compression level, progressive flag, etc.
//!
//! Kept as a single bundle so the CLI, dialogs and "Para web" presets can
//! build it once and pass a reference through the rest of the pipeline
//! without every call site exploding into positional arguments.

/// Output encoder configuration. `Default` produces library-default
/// settings (what `image::save_with_format` would emit on its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncodeOptions {
    /// Quality `1..=100` for lossy formats (JPEG/WebP lossy). `None`
    /// falls back to the encoder's default (JPEG uses 75).
    pub quality: Option<u8>,
    /// Emit progressive JPEG (multi-pass). Accepted for now but the pure-
    /// Rust `image` encoder only writes baseline JPEGs — a future toggle
    /// to `mozjpeg-sys` will honour this flag. For other formats it's a
    /// no-op.
    pub progressive: bool,
    /// Apply stronger compression / cleanup: PNG uses the `Best`
    /// compression level; extra passes over metadata strip anything
    /// non-essential when we own that code path.
    pub optimize: bool,
}

impl EncodeOptions {
    /// Convenience: a preset tailored for web uploads (WhatsApp, general
    /// email). JPEG quality 85 with progressive + optimize.
    pub fn web_preset() -> Self {
        Self { quality: Some(85), progressive: true, optimize: true }
    }

    /// Returns the resolved JPEG quality — user value when set, otherwise
    /// the library default (75).
    pub fn jpeg_quality(&self) -> u8 {
        self.quality.unwrap_or(75)
    }
}
