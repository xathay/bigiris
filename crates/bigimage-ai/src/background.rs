//! Background removal — wraps a BiRefNet-lite ONNX model to produce an
//! alpha mask, then composites it onto the original to yield an RGBA
//! image whose background is transparent.
//!
//! **Feature gate:** real inference lives behind `onnx`. Without the
//! feature, [`remove_background`] returns a clear error so the CLI /
//! dialog can offer a "rebuild with `--features ai` / install
//! `onnxruntime`" message instead of silently failing.
//!
//! Licensing: BiRefNet ships under MIT. We refuse any model whose SPDX
//! identifier isn't on the FOSS allowlist in [`super::download`].

use image::DynamicImage;
#[cfg(feature = "onnx")]
use image::{GenericImageView, ImageBuffer, Rgba};
use thiserror::Error;

#[cfg(feature = "onnx")]
use crate::download;
use crate::download::{DownloadError, ModelSource};

/// BiRefNet-lite ONNX — mirror oficial da comunidade ONNX, exportado a
/// partir do repositório canônico [`ZhengPeng7/BiRefNet_lite`] (MIT)
/// pela equipe Xenova / onnx-community. Revisão fixa via `resolve/main`;
/// o hash abaixo cobre o `.onnx` FP32 (~214 MB) tal como publicado em
/// 2024-09. Se o mirror re-envelopar os pesos, a verificação em
/// [`super::download::ensure`] rejeita o arquivo antes de gravá-lo na
/// cache do usuário.
pub const BIREFNET_LITE: ModelSource = ModelSource {
    id: "birefnet-lite",
    url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/main/onnx/model.onnx",
    license_spdx: "MIT",
    sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
    size_bytes: 224_005_088,
    description: "BiRefNet-lite — remoção de fundo FP32, MIT (onnx-community mirror).",
};

/// Progress signal emitted by [`remove_background_with_progress`]. The
/// UI layer uses it to drive a `gtk::ProgressBar`; the CLI discards it.
/// Both variants are `Copy` so the callback can be `FnMut(BgStage)`
/// without borrowing gymnastics in worker threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgStage {
    /// Downloading the ONNX weights. First call only; hash-verified cache
    /// hits skip this phase entirely. `total` may be 0 briefly if the
    /// server didn't send `Content-Length` — callers should guard.
    Download {
        /// Bytes downloaded so far.
        done: u64,
        /// Expected total bytes (from `Content-Length` or the catalogue
        /// `size_bytes`; 0 if neither is available).
        total: u64,
    },
    /// Weights on disk, inference running. Atomic from the UI's POV —
    /// we don't get intermediate signal out of BiRefNet — so we emit
    /// this once right before `session.run()` and call it a day.
    Infer,
}

/// Errors that can escape [`remove_background`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BgError {
    /// The `onnx` cargo feature wasn't enabled at build time — no
    /// inference backend to run.
    #[error(
        "feature `onnx` desabilitada nesta build. Recompile com `cargo build --features ai` \
         e instale `onnxruntime` (ArchLinux: `pacman -S onnxruntime`)."
    )]
    OnnxDisabled,
    /// Downloading / caching the model failed.
    #[error("modelo: {0}")]
    Model(#[from] DownloadError),
    /// ORT / inference backend error.
    #[error("inferência: {0}")]
    Inference(String),
    /// Preprocessing / postprocessing error.
    #[error("processamento: {0}")]
    Processing(String),
}

/// Remove the background of `input`, returning an RGBA image whose
/// alpha channel tracks the foreground mask predicted by BiRefNet.
///
/// Does not touch disk: pure in-memory, safe to call from dialogs on
/// the UI thread (though you probably want `spawn_blocking`).
pub fn remove_background(input: &DynamicImage) -> Result<DynamicImage, BgError> {
    remove_background_with_progress(input, |_| {})
}

/// Same as [`remove_background`], but forwards a [`BgStage`] signal at
/// each phase change so the caller can drive a progress bar. Download
/// bytes flow through as `BgStage::Download { done, total }`; the
/// transition to inference emits `BgStage::Infer` once.
pub fn remove_background_with_progress(
    input: &DynamicImage,
    mut progress: impl FnMut(BgStage),
) -> Result<DynamicImage, BgError> {
    #[cfg(not(feature = "onnx"))]
    {
        let _ = (input, &mut progress);
        Err(BgError::OnnxDisabled)
    }

    #[cfg(feature = "onnx")]
    {
        use image::imageops::FilterType;

        // 1. Ensure model is on disk (download if needed). The progress
        //    callback fires once per chunk during download and once with
        //    (size, size) when the cache is already warm.
        let model_path = download::ensure(&BIREFNET_LITE, |done, total| {
            progress(BgStage::Download { done, total });
        })?;
        progress(BgStage::Infer);

        // 2. Load / cache the ort session.
        let mut session = ort_session(&model_path)?;

        // 3. Preprocess: resize to 1024x1024, normalise with ImageNet
        //    mean/std, NCHW float tensor.
        let (orig_w, orig_h) = input.dimensions();
        let resized = input.resize_exact(1024, 1024, FilterType::Lanczos3).to_rgb8();

        let mean = [0.485f32, 0.456, 0.406];
        let std = [0.229f32, 0.224, 0.225];
        let mut tensor = ndarray::Array4::<f32>::zeros((1, 3, 1024, 1024));
        for y in 0..1024u32 {
            for x in 0..1024u32 {
                let p = resized.get_pixel(x, y).0;
                for c in 0..3 {
                    let v = (p[c] as f32 / 255.0 - mean[c]) / std[c];
                    tensor[[0, c, y as usize, x as usize]] = v;
                }
            }
        }

        // 4. Run inference.
        let mask = run_session(&mut session, tensor)?;

        // 5. Postprocess: resize mask back to original dims, composite.
        let mask_img: ImageBuffer<image::Luma<u8>, Vec<u8>> =
            ImageBuffer::from_fn(1024, 1024, |x, y| {
                let v = mask[[0, 0, y as usize, x as usize]];
                image::Luma([(v.clamp(0.0, 1.0) * 255.0) as u8])
            });
        let mask_full = image::imageops::resize(
            &mask_img,
            orig_w,
            orig_h,
            image::imageops::FilterType::Lanczos3,
        );

        let mut out: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(orig_w, orig_h);
        let src_rgba = input.to_rgba8();
        for y in 0..orig_h {
            for x in 0..orig_w {
                let src = src_rgba.get_pixel(x, y).0;
                let m = mask_full.get_pixel(x, y).0[0];
                out.put_pixel(x, y, Rgba([src[0], src[1], src[2], m]));
            }
        }
        Ok(DynamicImage::ImageRgba8(out))
    }
}

#[cfg(feature = "onnx")]
fn ort_session(path: &std::path::Path) -> Result<ort::session::Session, BgError> {
    ort::session::Session::builder()
        .and_then(|b| b.commit_from_file(path))
        .map_err(|e| BgError::Inference(format!("load session: {e}")))
}

#[cfg(feature = "onnx")]
fn run_session(
    session: &mut ort::session::Session,
    input: ndarray::Array4<f32>,
) -> Result<ndarray::Array4<f32>, BgError> {
    use ort::value::Tensor;

    let input_tensor =
        Tensor::from_array(input).map_err(|e| BgError::Processing(format!("tensor: {e}")))?;
    let inputs = ort::inputs![input_tensor];
    let outputs = session.run(inputs).map_err(|e| BgError::Inference(format!("run: {e}")))?;

    // BiRefNet normally emits a single (1,1,H,W) float tensor. Grab the
    // first one regardless of the exact output name so we survive minor
    // model revisions.
    let (_, first) =
        outputs.iter().next().ok_or_else(|| BgError::Inference("sem saidas".to_string()))?;
    let (shape, data) = first
        .try_extract_tensor::<f32>()
        .map_err(|e| BgError::Inference(format!("extract: {e}")))?;
    let shape_slice: &[i64] = shape;
    let dims: Vec<usize> = shape_slice.iter().map(|&d| d as usize).collect();
    if dims.len() != 4 {
        return Err(BgError::Processing(format!("saída não 4D: dims = {dims:?}")));
    }
    ndarray::Array4::<f32>::from_shape_vec((dims[0], dims[1], dims[2], dims[3]), data.to_vec())
        .map_err(|e| BgError::Processing(format!("reshape: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birefnet_lite_is_on_license_allowlist() {
        assert!(crate::download::allowed_licenses().contains(&BIREFNET_LITE.license_spdx));
    }

    #[cfg(not(feature = "onnx"))]
    #[test]
    fn without_onnx_returns_clear_error() {
        let img = DynamicImage::new_rgb8(8, 8);
        let err = remove_background(&img).unwrap_err();
        assert!(matches!(err, BgError::OnnxDisabled));
    }
}
