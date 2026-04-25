//! bigimage-ai — AI inference backend for BigIris.
//!
//! The model download manager (`download` module) is always built —
//! it's cheap and lets the CLI/dialog show accurate "baixar modelo?"
//! prompts even on builds without ONNX Runtime linked. Actual inference
//! (`background`) is feature-gated behind `onnx` so default builds
//! stay lean.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod background;
pub mod download;

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Whether the ONNX backend is compiled in.
pub fn onnx_available() -> bool {
    cfg!(feature = "onnx")
}

/// IA task identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Task {
    /// Background removal (BiRefNet / U²-Net).
    RemoveBackground,
    /// Super-resolution (Real-ESRGAN / Real-CUGAN).
    Upscale,
    /// Denoise (SCUNet).
    Denoise,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }
}
