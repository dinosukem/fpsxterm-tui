use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::{
    fs,
    io::AsyncWriteExt,
    sync::{mpsc, watch},
};

use super::temporary_path;
use crate::{sftp::RemoteFs, terminal::RuntimeEvent};

const SFTP_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[allow(clippy::too_many_arguments)]
pub async fn download_path(
    remote: &mut RemoteFs,
    remote_path: &Path,
    local: &Path,
    id: u64,
    tx: &mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Result<u64> {
    if *cancel.borrow() {
        bail!("transfer cancelled");
    }
    let metadata = tokio::time::timeout(
        SFTP_IDLE_TIMEOUT,
        remote.client().fs().symlink_metadata(remote_path),
    )
    .await
    .context("timed out reading remote file metadata")??;
    let kind = metadata.file_type();
    if kind.is_some_and(|kind| kind.is_symlink()) {
        bail!("symbolic links are skipped");
    }
    if kind.is_some_and(|kind| kind.is_dir()) {
        match fs::symlink_metadata(local).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!(
                "destination exists and is not a directory: {}",
                local.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(local)
                .await
                .with_context(|| format!("creating local directory {}", local.display()))?,
            Err(error) => return Err(error.into()),
        }
        let mut copied = 0;
        for entry in remote.list(remote_path).await? {
            if entry.kind == crate::filesystem::EntryKind::Symlink {
                continue;
            }
            copied += download_recursive(
                remote,
                &entry.path,
                &local.join(&entry.name),
                id,
                tx,
                cancel.clone(),
                progress.clone(),
                overwrite,
            )
            .await?;
        }
        return Ok(copied);
    }
    download_regular_file(
        remote,
        remote_path,
        local,
        id,
        tx,
        cancel,
        progress,
        overwrite,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
fn download_recursive<'a>(
    remote: &'a mut RemoteFs,
    remote_path: &'a Path,
    local: &'a Path,
    id: u64,
    tx: &'a mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Pin<Box<dyn Future<Output = Result<u64>> + Send + 'a>> {
    Box::pin(async move {
        download_path(
            remote,
            remote_path,
            local,
            id,
            tx,
            cancel,
            progress,
            overwrite,
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
async fn download_regular_file(
    remote: &mut RemoteFs,
    remote_path: &Path,
    local: &Path,
    id: u64,
    tx: &mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Result<u64> {
    if !overwrite && fs::symlink_metadata(local).await.is_ok() {
        bail!("destination already exists: {}", local.display());
    }
    let temporary = temporary_path(local);
    let mut input = tokio::time::timeout(SFTP_IDLE_TIMEOUT, remote.client().open(remote_path))
        .await
        .context("timed out opening remote file")?
        .context("opening remote file")?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await
        .context("creating local temporary file")?;
    let result = async {
        let mut copied = 0;
        loop {
            if *cancel.borrow() {
                bail!("transfer cancelled");
            }
            let Some(bytes) = tokio::time::timeout(
                SFTP_IDLE_TIMEOUT,
                input.read(64 * 1024, bytes::BytesMut::new()),
            )
            .await
            .context("timed out reading remote file")??
            else {
                break;
            };
            output.write_all(&bytes).await?;
            copied += bytes.len() as u64;
            let total =
                progress.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
            let _ = tx
                .send(RuntimeEvent::TransferProgress { id, bytes: total })
                .await;
        }
        tokio::time::timeout(SFTP_IDLE_TIMEOUT, input.close())
            .await
            .context("timed out closing remote file")??;
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        if overwrite {
            fs::rename(&temporary, local).await?;
        } else {
            // Linking is atomic and fails if the destination appeared after the
            // initial existence check, so a concurrent file is never replaced.
            fs::hard_link(&temporary, local).await.with_context(|| {
                format!(
                    "creating destination without overwriting {}",
                    local.display()
                )
            })?;
            fs::remove_file(&temporary).await?;
        }
        Ok::<u64, anyhow::Error>(copied)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temporary).await;
    }
    result
}
