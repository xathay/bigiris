// SPDX-License-Identifier: GPL-3.0-or-later
//! Async batch primitives shared across every Prisma dialog.
//!
//! The 7 simple per-file dialogs (Convert, Resize, Rotate, Flip, Crop,
//! Adjust, Upscale) each delegate their Apply handler to
//! [`run_batch_async`], which runs the per-file CPU op on a worker
//! thread and polls progress on the GTK main loop at 20 Hz. The
//! Animate and remove-bg flows do not use this helper because they
//! have one-shot or AI-specific shapes.
//!
//! Cancellation is wired through `connect_close_request` on the
//! window: clicking close flips an atomic flag the worker checks
//! between files, so an in-flight decode finishes but the remaining
//! batch is skipped cleanly.

use std::path::{Path, PathBuf};

use bigimage_core::ConvertOutcome;
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

/// Render the final state of a Prisma dialog after a batch finishes.
/// Re-enables the Apply / Cancel buttons, prints `ok / skip / fail`
/// counts (with the first error appended if any), and triggers a
/// 2-second auto-close on clean success so service-menu callers
/// don't need to dismiss the window manually.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_dialog(
    status: &gtk::Label,
    apply_btn: &gtk::Button,
    cancel_btn: &gtk::Button,
    window: &adw::ApplicationWindow,
    ok: usize,
    skip: usize,
    fail: usize,
    first_err: Option<String>,
) {
    let msg = format!("{ok} gravado(s), {skip} ignorado(s), {fail} falha(s)");
    status.set_text(&msg);
    if let Some(err) = first_err {
        status.set_text(&format!("{msg}\n{err}"));
    }
    apply_btn.set_sensitive(true);
    cancel_btn.set_sensitive(true);
    if fail == 0 {
        let window = window.clone();
        gtk4::glib::timeout_add_seconds_local_once(2, move || window.close());
    }
}

/// Progress message sent from the worker thread to the UI poll.
enum AsyncBatchEvent {
    /// About to process file `idx + 1` of `total`. `file` is the bare
    /// file name (no directory) for display.
    Tick { idx: usize, total: usize, file: String },
    /// Worker is done. Pass the same shape [`finish_dialog`] consumes.
    Done { ok: usize, skip: usize, fail: usize, first_err: Option<String> },
}

/// Run a per-file batch on a worker thread, polling progress on the main
/// loop at 20 Hz. The `work` closure is the per-file CPU op (e.g.
/// `bigimage_core::convert_file`); it must be `Send + 'static` so the
/// thread can take it. Widgets (`status`, buttons, window) stay on the
/// main thread — they're only touched inside the timeout closure, which
/// runs locally.
///
/// Cancellation: closing the window flips an atomic flag the worker
/// checks between files. The current file finishes (the decoder isn't
/// interruptible mid-flight today), but the rest of the batch is
/// skipped cleanly.
///
/// Replaces the previous `idle_add_local_once(|| run_X_batch(...);
/// finish_dialog(...))` pattern that ran the entire batch on the main
/// thread, freezing the UI for the duration. With this helper the user
/// can drag the window, scroll, see "Convertendo 12/50 — foo.jpg"
/// updating live, and close the dialog mid-batch without GTK marking
/// the app "Not responding".
#[allow(clippy::too_many_arguments)]
pub(super) fn run_batch_async<W>(
    files: Vec<PathBuf>,
    mut work: W,
    busy_label: String,
    status: gtk::Label,
    apply_btn: gtk::Button,
    cancel_btn: gtk::Button,
    window: adw::ApplicationWindow,
) where
    W: FnMut(&Path) -> bigimage_core::Result<ConvertOutcome> + Send + 'static,
{
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;

    let (tx, rx) = mpsc::channel::<AsyncBatchEvent>();
    let cancelled = Arc::new(AtomicBool::new(false));

    {
        let cancelled = cancelled.clone();
        window.connect_close_request(move |_| {
            cancelled.store(true, Ordering::Relaxed);
            gtk4::glib::Propagation::Proceed
        });
    }

    {
        let cancelled = cancelled.clone();
        std::thread::spawn(move || {
            let mut ok = 0usize;
            let mut skip = 0usize;
            let mut fail = 0usize;
            let mut first_err: Option<String> = None;
            let total = files.len();
            for (idx, f) in files.iter().enumerate() {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.send(AsyncBatchEvent::Tick {
                    idx,
                    total,
                    file: f
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                });
                match work(f) {
                    Ok(ConvertOutcome::Written { .. }) => ok += 1,
                    Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
                    Err(e) => {
                        fail += 1;
                        if first_err.is_none() {
                            first_err = Some(format!("{}: {e}", f.display()));
                        }
                    }
                }
            }
            let _ = tx.send(AsyncBatchEvent::Done { ok, skip, fail, first_err });
        });
    }

    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(50), move || loop {
        match rx.try_recv() {
            Ok(AsyncBatchEvent::Tick { idx, total, file }) => {
                status.set_text(&format!("{busy_label} {}/{} — {}", idx + 1, total, file));
            }
            Ok(AsyncBatchEvent::Done { ok, skip, fail, first_err }) => {
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
                return gtk4::glib::ControlFlow::Break;
            }
            Err(mpsc::TryRecvError::Empty) => return gtk4::glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => return gtk4::glib::ControlFlow::Break,
        }
    });
}
