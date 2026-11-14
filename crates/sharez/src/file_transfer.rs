//! Handing copied files to sandboxed apps through the document portal's `FileTransfer`.
//!
//! A Flatpak or Snap app that pastes `file:///home/…` can't open the path (Flathub Telegram says
//! the file "is empty", Flathub Discord does nothing). Offering a portal key as
//! `application/vnd.portal.filetransfer` lets it call `RetrieveFiles` and get a copy of the path
//! under `/run/user/<uid>/doc` that it may read.
//!
//! GTK offers such a key for a `GdkFileList` itself, but it ends the key with a NUL byte, which
//! receivers that read the data as a string pass on to `RetrieveFiles` and fail with, and it makes
//! the transfer stop after the first read. ShareZ starts its own transfer instead: one key, no
//! NUL, kept alive until something else takes the clipboard.

use std::{fs::File, path::PathBuf, time::Duration};

use ashpd::documents::file_transfer::{AddFilesOptions, FileTransfer, StartTransferOptions};
use gtk::glib;

/// How long ShareZ waits for the portal, so a stuck one can't hold up the copy.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Files per `AddFiles` call, to stay well under the open-file limit.
const BATCH: usize = 16;

/// Starts a transfer of `paths` and returns its key, or `None` if there is no document portal or
/// it didn't answer in time. Stop it with `stop` once the clipboard no longer offers it.
pub async fn start(paths: &[PathBuf]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    glib::future_with_timeout(TIMEOUT, register(paths))
        .await
        .map_err(|_| "timed out".to_owned())
        .and_then(|r| r)
        .inspect_err(|e| eprintln!("No portal file transfer for the clipboard: {e}"))
        .ok()
}

async fn register(paths: &[PathBuf]) -> Result<String, String> {
    let proxy = FileTransfer::new().await.map_err(|e| e.to_string())?;
    // Not `autostop`: some receivers read the key more than once, and an autostopped transfer
    // only answers the first. Read-only: pasting a recording never needs to change it.
    let key = proxy
        .start_transfer(
            StartTransferOptions::default()
                .set_writeable(false)
                .set_auto_stop(false),
        )
        .await
        .map_err(|e| e.to_string())?;
    for batch in paths.chunks(BATCH) {
        let added = async {
            // Regular files, so a plain read-only open can't block. Rust opens them close-on-exec.
            let files = batch
                .iter()
                .map(|p| File::open(p).map_err(|e| format!("{}: {e}", p.display())))
                .collect::<Result<Vec<_>, _>>()?;
            proxy
                .add_files(&key, &files, AddFilesOptions::default())
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(e) = added {
            let _ = proxy.stop_transfer(&key).await;
            return Err(e);
        }
    }
    Ok(key)
}

/// Ends a transfer `start` began, in the background.
pub fn stop(key: String) {
    glib::spawn_future_local(async move {
        let stopped = glib::future_with_timeout(TIMEOUT, async {
            FileTransfer::new().await?.stop_transfer(&key).await
        })
        .await;
        if let Ok(Err(e)) = stopped {
            eprintln!("Could not stop the clipboard's portal file transfer: {e}");
        }
    });
}
