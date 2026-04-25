// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-file conversion pipeline: decode → re-encode to a target format.
//!
//! The decode/encode/output-path primitives live in [`crate::pipeline`]; this
//! module just orchestrates them and owns the [`OverwritePolicy`] /
//! [`ConvertOutcome`] types shared across every transform module.

use std::path::{Path, PathBuf};

use image::DynamicImage;

use crate::{pipeline, Format, Result};

/// Policy to apply when the computed output path already exists on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverwritePolicy {
    /// Leave the existing file alone and return [`ConvertOutcome::Skipped`].
    #[default]
    Skip,
    /// Overwrite the destination unconditionally.
    Replace,
    /// Append `_1`, `_2`, … until a free name is found.
    Increment,
}

/// Result of a single-file conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertOutcome {
    /// File was written. The output path may differ from the naive one if
    /// [`OverwritePolicy::Increment`] had to disambiguate.
    Written {
        /// Path that now holds the converted image.
        output: PathBuf,
    },
    /// Destination already existed and policy was [`OverwritePolicy::Skip`].
    Skipped {
        /// Path that was left untouched.
        output: PathBuf,
    },
}

impl ConvertOutcome {
    /// Final path (written or skipped).
    pub fn path(&self) -> &Path {
        match self {
            ConvertOutcome::Written { output } | ConvertOutcome::Skipped { output } => output,
        }
    }
}

/// Convert a single file to `target` and write it next to the input using the
/// target's canonical extension.
///
/// The input is decoded with format auto-detection (by header when possible,
/// extension as fallback) and re-encoded into the target format. Colour space
/// and metadata preservation will land in a later iteration — today the
/// pipeline decodes into the image crate's internal 8-bit representation and
/// encodes straight back out.
pub fn convert_file(
    input: impl AsRef<Path>,
    target: Format,
    opts: &crate::EncodeOptions,
    policy: OverwritePolicy,
) -> Result<ConvertOutcome> {
    convert_file_to(input, None, target, opts, policy)
}

/// Like [`convert_file`], but honours `output_dir` when `Some` — writes
/// the converted file into that directory keeping the source stem and
/// honouring the overwrite policy (Skip/Replace/Increment). Passing
/// `None` is equivalent to [`convert_file`] (grava ao lado do original).
pub fn convert_file_to(
    input: impl AsRef<Path>,
    output_dir: Option<&Path>,
    target: Format,
    opts: &crate::EncodeOptions,
    policy: OverwritePolicy,
) -> Result<ConvertOutcome> {
    let input = input.as_ref();
    let output = pipeline::resolve_output_to(input, output_dir, None, target, policy)?;

    if matches!(policy, OverwritePolicy::Skip) && output.exists() {
        tracing::debug!(?output, "convert: skipping existing output");
        return Ok(ConvertOutcome::Skipped { output });
    }

    tracing::debug!(?input, ?output, ?target, ?opts, "convert: decoding");
    let (img, _src_format) = pipeline::decode_with_source_format(input)?;
    let output = pipeline::encode_and_cleanup(img, output, target, opts)?;
    Ok(ConvertOutcome::Written { output })
}

/// Coerce a `DynamicImage` into a colour/bit-depth that the target encoder is
/// willing to accept, avoiding encoder-side `UnsupportedError` from formats
/// that reject common inputs (OpenEXR won't take 8-bit integer; HDR wants
/// RGB floats).
pub(crate) fn prepare_for_target(img: DynamicImage, target: Format) -> DynamicImage {
    match target {
        Format::OpenExr => DynamicImage::ImageRgba32F(img.into_rgba32f()),
        Format::Hdr => DynamicImage::ImageRgb32F(img.into_rgb32f()),
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BigImageError;
    use image::{Rgb, RgbImage};
    use tempfile::TempDir;

    fn write_fixture_png(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut img = RgbImage::new(8, 8);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = Rgb([x as u8 * 32, y as u8 * 32, 128]);
        }
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn png_to_jpeg_writes_sibling_file() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");

        let outcome = convert_file(
            &src,
            Format::Jpeg,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap();
        let out = match outcome {
            ConvertOutcome::Written { output } => output,
            ConvertOutcome::Skipped { .. } => panic!("unexpected skip"),
        };

        assert_eq!(out, dir.path().join("fx.jpg"));
        assert!(out.exists());
        // Round-trip decodes cleanly.
        image::open(&out).unwrap();
    }

    #[test]
    fn skip_policy_preserves_existing_output() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");
        let dest = dir.path().join("fx.jpg");
        std::fs::write(&dest, b"sentinel-not-an-image").unwrap();

        let outcome = convert_file(
            &src,
            Format::Jpeg,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Skip,
        )
        .unwrap();
        assert!(matches!(outcome, ConvertOutcome::Skipped { .. }));
        // Sentinel bytes are still there — we didn't decode/re-encode over it.
        assert_eq!(std::fs::read(&dest).unwrap(), b"sentinel-not-an-image");
    }

    #[test]
    fn increment_policy_appends_suffix() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");
        std::fs::write(dir.path().join("fx.jpg"), b"prev").unwrap();

        let outcome = convert_file(
            &src,
            Format::Jpeg,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Increment,
        )
        .unwrap();
        let out = match outcome {
            ConvertOutcome::Written { output } => output,
            _ => panic!("expected write"),
        };
        assert_eq!(out, dir.path().join("fx_1.jpg"));
        assert!(out.exists());
        // Pre-existing sibling was left alone.
        assert_eq!(std::fs::read(dir.path().join("fx.jpg")).unwrap(), b"prev");
    }

    #[test]
    fn replace_policy_overwrites() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");
        let dest = dir.path().join("fx.jpg");
        std::fs::write(&dest, b"prev").unwrap();

        let outcome = convert_file(
            &src,
            Format::Jpeg,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap();
        assert!(matches!(outcome, ConvertOutcome::Written { .. }));
        // File must now be a real JPEG, not the placeholder.
        image::open(&dest).unwrap();
    }

    #[test]
    fn unknown_input_returns_decode_error() {
        let dir = TempDir::new().unwrap();
        let bogus = dir.path().join("not-an-image.png");
        std::fs::write(&bogus, b"garbage").unwrap();

        let err = convert_file(
            &bogus,
            Format::Jpeg,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap_err();
        assert!(matches!(err, BigImageError::Decode { .. }));
    }

    #[test]
    fn png_to_exr_auto_converts_to_float() {
        // EXR rejects 8-bit integer inputs; the prepare step must upcast.
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");

        let outcome = convert_file(
            &src,
            Format::OpenExr,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap();
        let out = match outcome {
            ConvertOutcome::Written { output } => output,
            _ => panic!("expected write"),
        };
        assert!(out.exists());
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
        image::open(&out).unwrap();
    }

    #[test]
    fn png_to_hdr_auto_converts_to_rgb_float() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");

        let outcome = convert_file(
            &src,
            Format::Hdr,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap();
        assert!(matches!(outcome, ConvertOutcome::Written { .. }));
    }

    #[test]
    fn png_to_qoi_and_back() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");

        let q = convert_file(
            &src,
            Format::Qoi,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap();
        let qoi_path = match q {
            ConvertOutcome::Written { output } => output,
            _ => panic!("expected write"),
        };
        let back = convert_file(
            &qoi_path,
            Format::Png,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Increment,
        )
        .unwrap();
        assert!(matches!(back, ConvertOutcome::Written { .. }));
    }

    #[test]
    fn png_to_avif_roundtrip() {
        let dir = TempDir::new().unwrap();
        let src = write_fixture_png(dir.path(), "fx.png");

        let a = convert_file(
            &src,
            Format::Avif,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Replace,
        )
        .unwrap();
        let avif_path = match a {
            ConvertOutcome::Written { output } => output,
            _ => panic!("expected write"),
        };
        // Decode the AVIF back out via the pipeline to confirm dav1d is wired.
        let back = convert_file(
            &avif_path,
            Format::Png,
            &crate::EncodeOptions::default(),
            OverwritePolicy::Increment,
        )
        .unwrap();
        assert!(matches!(back, ConvertOutcome::Written { .. }));
    }
}
