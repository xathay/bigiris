// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared primitives used by every single-file transform pipeline
//! (convert / resize / rotate / flip / crop). Extracted here so the op
//! modules stay focused on the transform itself and pick up the same
//! decode, naming, and fail-safe encoding semantics for free.

use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageReader};

use crate::convert::OverwritePolicy;
use crate::encode::EncodeOptions;
use crate::{BigImageError, Format, Result};

/// Hard cap on input file size before we even open the decoder. Beyond this
/// no normal photo workflow makes sense, and a malicious file (decode bomb,
/// ZIP-style amplification) would only cost more memory. 1 GiB is generous
/// enough for RAW or large EXR while still bounding the worst case.
const MAX_INPUT_BYTES: u64 = 1024 * 1024 * 1024;

/// Hard cap on decoded pixel count. 256 megapixels covers any consumer
/// camera (16384×16384 ≈ 268 MP) and most stitched panoramas; beyond it,
/// expanding to RGBA8 would allocate >1 GiB and risk DoS. Refused before
/// `decode()` so the malicious input never gets decompressed at all.
const MAX_PIXELS: u64 = 256 * 1024 * 1024;

/// Open `input`, identify its source format, and decode. Returns the decoded
/// image alongside the format tag so callers can preserve the source encoding
/// when the user didn't pick a target.
///
/// Refuses inputs that exceed [`MAX_INPUT_BYTES`] (file size) or
/// [`MAX_PIXELS`] (decoded dimensions), turning what would otherwise be
/// silent OOMs / multi-GB allocations into a clean error early in the
/// pipeline. The dimension check uses [`ImageReader::into_dimensions`]
/// which only parses the format header — no decompression.
pub(crate) fn decode_with_source_format(input: &Path) -> Result<(DynamicImage, Format)> {
    // Cheap first guard: file size from the FS metadata. Catches the
    // "attacker streamed 50 GB of garbage" case without touching the
    // decoder.
    let bytes = std::fs::metadata(input).map_err(BigImageError::Io)?.len();
    if bytes > MAX_INPUT_BYTES {
        return Err(BigImageError::InvalidInput(format!(
            "{} excede o limite de entrada ({} MB > {} MB)",
            input.display(),
            bytes / (1024 * 1024),
            MAX_INPUT_BYTES / (1024 * 1024),
        )));
    }

    // Peek dimensions before allocating a full decoded buffer. Refuses
    // pixel bombs (e.g. a tiny PNG header claiming 100k × 100k) before the
    // decoder ever expands them in memory.
    let dims_reader = ImageReader::open(input)
        .map_err(BigImageError::Io)?
        .with_guessed_format()
        .map_err(BigImageError::Io)?;
    let (w, h) = dims_reader
        .into_dimensions()
        .map_err(|e| BigImageError::Decode { path: input.to_path_buf(), source: e })?;
    let pixels = u64::from(w).saturating_mul(u64::from(h));
    if pixels > MAX_PIXELS {
        return Err(BigImageError::InvalidInput(format!(
            "{} excede o limite de pixels ({}×{} = {} MP > {} MP)",
            input.display(),
            w,
            h,
            pixels / (1024 * 1024),
            MAX_PIXELS / (1024 * 1024),
        )));
    }

    // Re-open for the actual decode. Cheap because the file is already in
    // the page cache from the dimension peek.
    let reader = ImageReader::open(input)
        .map_err(BigImageError::Io)?
        .with_guessed_format()
        .map_err(BigImageError::Io)?;

    let src_format = reader.format().and_then(Format::from_image_format).ok_or_else(|| {
        BigImageError::UnsupportedFormat(format!(
            "entrada sem formato reconhecido: {}",
            input.display()
        ))
    })?;

    let img = reader
        .decode()
        .map_err(|e| BigImageError::Decode { path: input.to_path_buf(), source: e })?;

    Ok((img, src_format))
}

/// Compute the output path.
///
/// * `suffix`: optional tag appended to the stem (e.g. `"_1080"`, `"_rot90"`).
///   `None` means "same stem" (convert's behaviour).
/// * `policy`: applied only when it's `Increment` and the naive path is taken;
///   caller still decides whether to re-check `.exists()` for `Skip`.
pub(crate) fn resolve_output(
    input: &Path,
    suffix: Option<&str>,
    target: Format,
    policy: OverwritePolicy,
) -> Result<PathBuf> {
    resolve_output_to(input, None, suffix, target, policy)
}

/// Like [`resolve_output`] but takes an explicit `output_dir`. If `None`,
/// grava ao lado do arquivo original (comportamento histórico). Caso
/// contrário usa o diretório fornecido mantendo o stem da fonte.
pub(crate) fn resolve_output_to(
    input: &Path,
    output_dir: Option<&Path>,
    suffix: Option<&str>,
    target: Format,
    policy: OverwritePolicy,
) -> Result<PathBuf> {
    let stem = input
        .file_stem()
        .ok_or_else(|| BigImageError::InvalidInput(format!("no file stem in {input:?}")))?;
    let parent = output_dir.or_else(|| input.parent()).unwrap_or_else(|| Path::new("."));
    let ext = target.extension();
    let suffix = suffix.unwrap_or("");

    let naive = {
        let mut name = std::ffi::OsString::from(stem);
        name.push(suffix);
        name.push(".");
        name.push(ext);
        parent.join(name)
    };

    if !matches!(policy, OverwritePolicy::Increment) || !naive.exists() {
        return Ok(naive);
    }

    for n in 1u32..=9_999 {
        let mut name = std::ffi::OsString::from(stem);
        name.push(suffix);
        name.push(format!("_{n}."));
        name.push(ext);
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(BigImageError::Other(format!("too many existing siblings of {naive:?}")))
}

/// Encode `img` to `output` using `target`, coercing colour depth if the
/// encoder is picky (see [`prepare_for_target`]) and removing any half-written
/// file on failure.
///
/// `opts` controls quality and optimise knobs for formats that honour them
/// (currently JPEG quality and PNG compression level). Formats with no
/// tunable parameters ignore `opts` entirely.
///
/// [`prepare_for_target`]: crate::convert::prepare_for_target
pub(crate) fn encode_and_cleanup(
    img: DynamicImage,
    output: PathBuf,
    target: Format,
    opts: &EncodeOptions,
) -> Result<PathBuf> {
    let prepared = crate::convert::prepare_for_target(img, target);

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(BigImageError::Io)?;
        }
    }

    let result = encode_to_path(&prepared, &output, target, opts);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&output);
        return Err(BigImageError::Encode { path: output, source: e });
    }

    Ok(output)
}

/// Do the actual write, picking a format-specific encoder when we need to
/// honour `opts`, and falling back to `save_with_format` for formats where
/// `image` gives us no extra knobs.
fn encode_to_path(
    img: &DynamicImage,
    output: &Path,
    target: Format,
    opts: &EncodeOptions,
) -> std::result::Result<(), image::ImageError> {
    match target {
        Format::Jpeg if opts.quality.is_some() => {
            // Custom quality path — bypass save_with_format's default 75
            // and emit via a manual encoder call instead.
            use image::codecs::jpeg::JpegEncoder;
            use image::ImageEncoder;
            let file = std::fs::File::create(output)?;
            let mut writer = std::io::BufWriter::new(file);
            let quality = opts.jpeg_quality();
            let encoder = JpegEncoder::new_with_quality(&mut writer, quality);
            let rgb = img.to_rgb8();
            encoder.write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )?;
            Ok(())
        }
        Format::Avif => {
            // `image::save_with_format(Avif, …)` calls
            // `AvifEncoder::new(writer)` which defaults to speed=4. On
            // screenshots that's ~5–10× slower than speed=8, turning a
            // 100-file batch into half an hour of waiting. Drive the
            // encoder directly so the EncodeOptions::avif_speed knob
            // (default 8, or 4 when `optimize=true`) actually reaches
            // ravif. AVIF is encoded from RGBA to preserve the alpha
            // channel screenshots routinely have.
            use image::codecs::avif::AvifEncoder;
            use image::ImageEncoder;
            let file = std::fs::File::create(output)?;
            let mut writer = std::io::BufWriter::new(file);
            let speed = opts.effective_avif_speed();
            let quality = opts.quality.unwrap_or(80);
            let encoder = AvifEncoder::new_with_speed_quality(&mut writer, speed, quality);
            let rgba = img.to_rgba8();
            encoder.write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(())
        }
        Format::Png if opts.optimize => {
            use image::codecs::png::{CompressionType, FilterType, PngEncoder};
            use image::ImageEncoder;
            let file = std::fs::File::create(output)?;
            let mut writer = std::io::BufWriter::new(file);
            let encoder = PngEncoder::new_with_quality(
                &mut writer,
                CompressionType::Best,
                FilterType::Adaptive,
            );
            let rgba = img.to_rgba8();
            encoder.write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(())
        }
        _ => img.save_with_format(output, target.to_image_format()),
    }
}
