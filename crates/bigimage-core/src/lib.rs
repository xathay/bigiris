// SPDX-License-Identifier: GPL-3.0-or-later
//! bigimage-core — shared image-processing library.
//!
//! Brand-agnostic on purpose: this crate must stay usable if the project ever
//! splits into multiple binaries. It owns decode/encode/transform pipelines;
//! GUI code never lives here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use thiserror::Error;

pub mod adjust;
pub mod animate;
mod convert;
pub mod crop;
mod encode;
pub mod flip;
mod format;
pub mod metadata;
mod pipeline;
pub mod preview;
pub mod resize;
pub mod rotate;

pub use adjust::{adjust_file, AdjustOps};
pub use animate::{make_gif, AnimateOptions, LoopMode};
pub use convert::{convert_file, convert_file_to, ConvertOutcome, OverwritePolicy};
pub use crop::{crop_file, CropRect};
pub use encode::EncodeOptions;
pub use flip::{flip_file, FlipAxis};
pub use format::Format;
pub use metadata::Metadata;
pub use preview::{PreviewOp, PreviewSession};
pub use resize::{resize_file, Filter, ResizeMode};
pub use rotate::{rotate_file, rotate_file_auto, Rotation};

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Errors returned by core pipelines.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BigImageError {
    /// IO failure.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),

    /// User-supplied argument or path is malformed.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Requested format is not implemented.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Decoder failed on a given file.
    #[error("decode failed for {path}: {source}")]
    Decode {
        /// Path of the file that failed to decode.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: image::ImageError,
    },

    /// Encoder failed while writing a target.
    #[error("encode failed for {path}: {source}")]
    Encode {
        /// Destination path we were writing to.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: image::ImageError,
    },

    /// Pipeline cancelled by caller.
    #[error("cancelled")]
    Cancelled,

    /// Catch-all for messages not yet promoted to variants.
    #[error("{0}")]
    Other(String),
}

/// Alias for pipeline results.
pub type Result<T> = std::result::Result<T, BigImageError>;

/// One conversion job, input to output. Left intentionally thin in M1 —
/// batch orchestration (`Pipeline` trait with progress + cancel) will layer
/// on top once more than a single transform type exists.
#[derive(Debug, Clone)]
pub struct ConvertJob {
    /// Source path.
    pub input: PathBuf,
    /// Destination path (resolved by naming strategy upstream).
    pub output: PathBuf,
    /// Target format.
    pub target_format: Format,
}

impl ConvertJob {
    /// Create a new job with input, output, and target format.
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        target_format: Format,
    ) -> Self {
        Self { input: input.into(), output: output.into(), target_format }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn convert_job_roundtrip() {
        let job = ConvertJob::new("a.jpg", "a.png", Format::Png);
        assert_eq!(job.target_format, Format::Png);
    }
}
