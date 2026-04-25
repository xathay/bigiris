// SPDX-License-Identifier: GPL-3.0-or-later
//! Encoder-side tuning knobs shared across every `*_file` pipeline:
//! JPEG quality, PNG compression level, progressive flag, etc.
//!
//! Kept as a single bundle so the CLI, dialogs and "Para web" presets can
//! build it once and pass a reference through the rest of the pipeline
//! without every call site exploding into positional arguments.

/// Output encoder configuration.
///
/// `Default` produces library-default settings *plus* `strip_metadata =
/// true` (privacy-by-default contract). To change quality/progressive/
/// etc. in tests or call sites, prefer `..EncodeOptions::default()` so
/// new fields don't silently regress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeOptions {
    /// Quality `1..=100` for lossy formats (JPEG/WebP lossy). `None`
    /// falls back to the encoder's default (JPEG uses 75).
    pub quality: Option<u8>,
    /// Emit progressive JPEG (multi-pass). Accepted for now but the pure-
    /// Rust `image` encoder only writes baseline JPEGs — a future toggle
    /// to `mozjpeg-sys` will honour this flag. For other formats it's a
    /// no-op.
    pub progressive: bool,
    /// Apply stronger compression: PNG uses the `Best` zlib level
    /// (slower encode, smaller file). Lossless — the same pixels.
    pub optimize: bool,
    /// Strip EXIF/IPTC/XMP/GPS on encode. **Default `true`.** Today our
    /// pipeline already drops metadata as a side-effect of decoding to
    /// `DynamicImage` (which has no metadata carrier) before re-encoding,
    /// so this flag is documentation of intent — but it's the contract a
    /// future "preserve metadata" feature must override explicitly. The
    /// regression test in `tests::default_options_strip_metadata` exists
    /// to fail if the default ever silently flips to `false`.
    pub strip_metadata: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self { quality: None, progressive: false, optimize: false, strip_metadata: true }
    }
}

impl EncodeOptions {
    /// Convenience: a preset tailored for web uploads (WhatsApp, general
    /// email). JPEG quality 85 with progressive + optimize. Metadata
    /// stripped (default).
    pub fn web_preset() -> Self {
        Self { quality: Some(85), progressive: true, optimize: true, ..Self::default() }
    }

    /// Returns the resolved JPEG quality — user value when set, otherwise
    /// the library default (75).
    pub fn jpeg_quality(&self) -> u8 {
        self.quality.unwrap_or(75)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Privacy-by-default: the bare `EncodeOptions::default()` MUST strip
    /// EXIF/GPS/IPTC. Failing this test means a code path will leak
    /// metadata that today gets dropped silently — usually GPS coords.
    #[test]
    fn default_options_strip_metadata() {
        assert!(EncodeOptions::default().strip_metadata);
    }

    #[test]
    fn web_preset_strips_metadata() {
        assert!(EncodeOptions::web_preset().strip_metadata);
    }
}
