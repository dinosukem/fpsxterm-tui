use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::sftp::RemoteFs;
use crate::terminal::RuntimeEvent;
use tokio::sync::mpsc;
use tokio::sync::watch;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_path(destination: &Path) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    destination.with_file_name(format!(".{name}.fpsxterm-{id}.part"))
}

#[allow(clippy::too_many_arguments)]
pub async fn upload_path(
    remote: &mut RemoteFs,
    local: &Path,
    remote_path: &Path,
    id: u64,
    tx: &mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Result<u64> {
    if *cancel.borrow() {
        bail!("transfer cancelled");
    }
    let metadata = fs::symlink_metadata(local).await?;
    if metadata.file_type().is_symlink() {
        bail!("symbolic links are skipped");
    }
    if metadata.is_dir() {
        remote.ensure_dir(remote_path).await?;
        let mut copied = 0;
        for entry in crate::filesystem::list_local(local).await? {
            if entry.kind == crate::filesystem::EntryKind::Symlink {
                continue;
            }
            copied += upload_recursive(
                remote,
                &entry.path,
                &remote_path.join(&entry.name),
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
    if !metadata.is_file() {
        bail!("only regular files and directories can be transferred");
    }
    upload_regular_file(
        remote,
        local,
        remote_path,
        id,
        tx,
        cancel,
        progress,
        overwrite,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
fn upload_recursive<'a>(
    remote: &'a mut RemoteFs,
    local: &'a Path,
    remote_path: &'a Path,
    id: u64,
    tx: &'a mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Pin<Box<dyn Future<Output = Result<u64>> + Send + 'a>> {
    Box::pin(async move {
        upload_path(
            remote,
            local,
            remote_path,
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
async fn upload_regular_file(
    remote: &mut RemoteFs,
    local: &Path,
    remote_path: &Path,
    id: u64,
    tx: &mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Result<u64> {
    let mut input = fs::File::open(local).await?;
    let temporary = temporary_path(remote_path);
    let mut options = remote.client().options();
    let mut output = options
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await
        .context("creating remote temporary file")?;
    let result = async {
        let mut copied = 0;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            if *cancel.borrow() {
                bail!("transfer cancelled");
            }
            let count = input.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count]).await?;
            copied += count as u64;
            let total = progress.fetch_add(count as u64, Ordering::Relaxed) + count as u64;
            let _ = tx
                .send(RuntimeEvent::TransferProgress { id, bytes: total })
                .await;
        }
        output.close().await?;
        if overwrite {
            remote.replace_path(&temporary, remote_path).await?;
        } else {
            remote.rename_path(&temporary, remote_path).await?;
        }
        Ok::<u64, anyhow::Error>(copied)
    }
    .await;
    if result.is_err() {
        let _ = remote
            .delete(&temporary, crate::filesystem::EntryKind::File)
            .await;
    }
    result
}

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
    let metadata = remote.client().fs().symlink_metadata(remote_path).await?;
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
    let mut input = remote
        .client()
        .open(remote_path)
        .await
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
            let Some(bytes) = input.read(64 * 1024, bytes::BytesMut::new()).await? else {
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
        input.close().await?;
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        fs::rename(&temporary, local).await?;
        Ok::<u64, anyhow::Error>(copied)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temporary).await;
    }
    result
}
