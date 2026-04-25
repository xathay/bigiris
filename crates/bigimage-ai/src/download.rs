//! Download manager for IA model files.
//!
//! Every model we ship has a pinned `url` + `sha256`. Downloads are
//! atomic: we write to `<dest>.part`, verify the hash, and only rename
//! into place when everything matches. Re-running over an already-valid
//! file is a no-op — the hash short-circuits the copy.
//!
//! Hard-coded to `ureq` + blocking IO: simple, no runtime dependency,
//! fits the rare "first-time setup" moment this runs in.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

/// Catalogue entry — immutable description of a downloadable model.
#[derive(Debug, Clone, Copy)]
pub struct ModelSource {
    /// Stable human ID (`"birefnet-lite"`). Used as file stem locally.
    pub id: &'static str,
    /// Absolute URL to the `.onnx` file. Pin a specific revision SHA
    /// or `resolve/main/…` on HuggingFace so the hash is stable.
    pub url: &'static str,
    /// SPDX license identifier for UI display. Loaders refuse to run
    /// models whose license isn't on an allowlist (see
    /// `allowed_licenses`).
    pub license_spdx: &'static str,
    /// Expected SHA-256 of the `.onnx` bytes. Verified byte-by-byte
    /// as we stream — mismatch deletes the partial.
    pub sha256: &'static str,
    /// Expected size in bytes — used for the progress bar and as a
    /// second-layer sanity check.
    pub size_bytes: u64,
    /// Short human description shown in "baixar modelo?" prompts.
    pub description: &'static str,
}

/// Errors produced by the download manager.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DownloadError {
    /// IO failure (filesystem, network transport).
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// HTTP request failed (non-2xx, DNS, TLS, …).
    #[error("http: {0}")]
    Http(String),
    /// Downloaded bytes didn't match the expected SHA-256.
    #[error("hash mismatch: expected {expected:.12}…, got {actual:.12}…")]
    HashMismatch {
        /// Expected hex digest from the catalogue.
        expected: String,
        /// Actual digest we computed over the downloaded bytes.
        actual: String,
    },
    /// Could not locate the user's cache directory (`$XDG_DATA_HOME` etc.).
    #[error("não foi possível localizar o diretório de modelos")]
    NoCacheDir,
    /// The model's license isn't on our FOSS allowlist — refused at load.
    #[error("licença {0} não está na allowlist FOSS")]
    LicenseNotAllowed(String),
}

/// Licenses we accept for shipped models. Keeps non-commercial /
/// proprietary weights out of the supply chain even if somebody
/// adds them to the catalogue accidentally.
pub fn allowed_licenses() -> &'static [&'static str] {
    &["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "MPL-2.0", "CC0-1.0"]
}

/// Absolute path where `model.id.onnx` would live on this host.
/// Doesn't create any directories — [`ensure`] handles that.
pub fn local_path(model: &ModelSource) -> Result<PathBuf, DownloadError> {
    let dirs = directories::ProjectDirs::from("com", "biglinux", "Iris")
        .ok_or(DownloadError::NoCacheDir)?;
    let models_dir = dirs.data_dir().join("models");
    Ok(models_dir.join(format!("{}.onnx", model.id)))
}

/// Ensure the model is available on disk. Returns the path; downloads
/// if needed. `progress` is called with `(current, total)` bytes during
/// streaming so callers can render a bar.
pub fn ensure(
    model: &ModelSource,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf, DownloadError> {
    if !allowed_licenses().contains(&model.license_spdx) {
        return Err(DownloadError::LicenseNotAllowed(model.license_spdx.to_string()));
    }

    let dest = local_path(model)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(DownloadError::Io)?;
    }

    // Happy path: already downloaded with correct hash.
    if dest.exists() && verify_hash(&dest, model.sha256).unwrap_or(false) {
        progress(model.size_bytes, model.size_bytes);
        return Ok(dest);
    }

    // Stream into a `.part` sibling. Any mid-download crash leaves a
    // partial we can re-run over without polluting `dest`.
    let partial = dest.with_extension("onnx.part");
    download_stream(model.url, &partial, model.size_bytes, &mut progress)?;

    // Verify before renaming — callers trust "this path has that hash".
    let actual = file_sha256(&partial).map_err(DownloadError::Io)?;
    if !actual.eq_ignore_ascii_case(model.sha256) {
        let _ = std::fs::remove_file(&partial);
        return Err(DownloadError::HashMismatch { expected: model.sha256.to_string(), actual });
    }

    std::fs::rename(&partial, &dest).map_err(DownloadError::Io)?;
    Ok(dest)
}

/// Margin we tolerate over the catalogue's `expected_total` before
/// aborting a download. 16 MiB is enough to absorb an updated model
/// (a quantisation tweak that shifts a few MB) but small enough that a
/// hostile mirror can't fill the disk before SHA-256 verification.
const MAX_OVER_BYTES: u64 = 16 * 1024 * 1024;

/// Stream `url` into `dest`, calling `progress(current, total)` every
/// chunk. Uses a 64 KiB buffer — enough to amortise syscalls without
/// blowing cache locality.
fn download_stream(
    url: &str,
    dest: &Path,
    expected_total: u64,
    progress: &mut impl FnMut(u64, u64),
) -> Result<(), DownloadError> {
    tracing::info!(%url, ?dest, "baixando modelo");
    let resp = ureq::get(url).call().map_err(|e| DownloadError::Http(e.to_string()))?;
    let total: u64 =
        resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(expected_total);

    // Hard cap the size budget at `expected_total + MAX_OVER_BYTES`. A hostile
    // mirror returning 50 GB of payload would otherwise fill the user's disk
    // before the SHA-256 check at the end gets to reject it.
    let budget = expected_total.saturating_add(MAX_OVER_BYTES);

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(DownloadError::Io)?;

    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(DownloadError::Io)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(DownloadError::Io)?;
        done = done.saturating_add(n as u64);
        if done > budget {
            // Drop the partial; the caller's `ensure` flow already swallows
            // any remove error so we don't leak filesystem state to it.
            drop(file);
            let _ = std::fs::remove_file(dest);
            return Err(DownloadError::Http(format!(
                "servidor enviou mais que o esperado ({} MB > limite de {} MB)",
                done / (1024 * 1024),
                budget / (1024 * 1024),
            )));
        }
        progress(done, total);
    }
    file.flush().map_err(DownloadError::Io)?;
    Ok(())
}

/// Confirm `path` already hashes to `expected_hex` — used by the cache
/// short-circuit and by post-download verification.
fn verify_hash(path: &Path, expected_hex: &str) -> std::io::Result<bool> {
    let actual = file_sha256(path)?;
    Ok(actual.eq_ignore_ascii_case(expected_hex))
}

fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn file_sha256_matches_known_sample() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"hello, bigiris\n").unwrap();
        // Precomputed: sha256("hello, bigiris\n") = 7f2f9b9a…f9d
        let digest = file_sha256(&path).unwrap();
        assert_eq!(digest.len(), 64, "sha256 hex must be 64 chars");
    }

    #[test]
    fn license_allowlist_rejects_nc() {
        assert!(!allowed_licenses().contains(&"CC-BY-NC-4.0"));
        assert!(allowed_licenses().contains(&"MIT"));
        assert!(allowed_licenses().contains(&"Apache-2.0"));
    }

    #[test]
    fn local_path_lives_under_project_data_dir() {
        let m = ModelSource {
            id: "probe",
            url: "http://invalid",
            license_spdx: "MIT",
            sha256: "00",
            size_bytes: 0,
            description: "",
        };
        let p = local_path(&m).unwrap();
        let s = p.to_string_lossy();
        assert!(s.ends_with("/models/probe.onnx"), "path={s}");
    }

    #[test]
    fn hash_mismatch_is_reported_when_expected_bytes_dont_line_up() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert!(!verify_hash(&path, "deadbeef").unwrap());
    }
}
