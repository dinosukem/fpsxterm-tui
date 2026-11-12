use {
    super::temporary_path,
    crate::{
        docker_fs::DockerFs,
        event::RuntimeEvent,
        filesystem::{
            EntryKind,
            FileEntry,
        },
    },
    anyhow::{
        Context,
        Result,
        bail,
    },
    std::{
        future::Future,
        path::Path,
        pin::Pin,
        sync::{
            Arc,
            atomic::{
                AtomicU64,
                Ordering,
            },
        },
    },
    tokio::{
        fs,
        io::{
            AsyncReadExt,
            AsyncWriteExt,
        },
        sync::{
            mpsc,
            watch,
        },
    },
};

#[allow(clippy::too_many_arguments)]
pub async fn upload_path(
    remote: &DockerFs,
    local: &Path,
    destination: &Path,
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
        remote.ensure_dir(destination).await?;
        let mut copied = 0;
        for entry in crate::filesystem::list_local(local).await? {
            if entry.kind == EntryKind::Symlink {
                continue;
            }
            copied += upload_recursive(
                remote,
                &entry.path,
                &destination.join(&entry.name),
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
    let mut input = fs::File::open(local).await?;
    let mut child = remote
        .open_write(destination, overwrite, metadata.len())
        .await?;
    let mut output = child
        .stdin
        .take()
        .context("opening container upload stream")?;
    let mut copied = 0;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        if *cancel.borrow() {
            drop(output);
            let _ = child.wait_with_output().await;
            bail!("transfer cancelled");
        }
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).await?;
        copied += count as u64;
        report_progress(id, count, &progress, tx).await;
    }
    drop(output);
    let result = child.wait_with_output().await?;
    if !result.status.success() {
        bail!("{}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(copied)
}

#[allow(clippy::too_many_arguments)]
fn upload_recursive<'a>(
    remote: &'a DockerFs,
    local: &'a Path,
    destination: &'a Path,
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
            destination,
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
pub async fn download_path(
    remote: &DockerFs,
    remote_path: &Path,
    local: &Path,
    id: u64,
    tx: &mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Result<u64> {
    let parent = remote_path
        .parent()
        .context("container path has no parent")?;
    let name = remote_path
        .file_name()
        .context("container path has no name")?
        .to_string_lossy();
    let entry = remote
        .list(parent)
        .await?
        .into_iter()
        .find(|entry| entry.name == name)
        .context("container entry no longer exists")?;
    download_entry(remote, &entry, local, id, tx, cancel, progress, overwrite).await
}

#[allow(clippy::too_many_arguments)]
fn download_entry<'a>(
    remote: &'a DockerFs,
    entry: &'a FileEntry,
    local: &'a Path,
    id: u64,
    tx: &'a mpsc::Sender<RuntimeEvent>,
    cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
    overwrite: bool,
) -> Pin<Box<dyn Future<Output = Result<u64>> + Send + 'a>> {
    Box::pin(async move {
        if *cancel.borrow() {
            bail!("transfer cancelled");
        }
        if entry.kind == EntryKind::Symlink {
            bail!("symbolic links are skipped");
        }
        if entry.kind == EntryKind::Directory {
            match fs::symlink_metadata(local).await {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    bail!(
                        "destination exists and is not a directory: {}",
                        local.display()
                    )
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(local).await?;
                }
                Err(error) => return Err(error.into()),
            }
            let mut copied = 0;
            for child in remote.list(&entry.path).await? {
                if child.kind == EntryKind::Symlink {
                    continue;
                }
                copied += download_entry(
                    remote,
                    &child,
                    &local.join(&child.name),
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
        if entry.kind != EntryKind::File {
            bail!("only regular files and directories can be transferred");
        }
        if !overwrite && fs::symlink_metadata(local).await.is_ok() {
            bail!("destination already exists: {}", local.display());
        }
        let temporary = temporary_path(local);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .context("creating local temporary file")?;
        let result = async {
            let mut child = remote.open_read(&entry.path).await?;
            let mut input = child
                .stdout
                .take()
                .context("opening container download stream")?;
            let mut copied = 0;
            let mut buffer = vec![0; 64 * 1024];
            loop {
                if *cancel.borrow() {
                    child.kill().await.ok();
                    let _ = child.wait().await;
                    bail!("transfer cancelled");
                }
                let count = input.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                output.write_all(&buffer[..count]).await?;
                copied += count as u64;
                report_progress(id, count, &progress, tx).await;
            }
            drop(input);
            let result = child.wait_with_output().await?;
            if !result.status.success() {
                bail!("{}", String::from_utf8_lossy(&result.stderr).trim());
            }
            output.flush().await?;
            output.sync_all().await?;
            drop(output);
            if overwrite {
                fs::rename(&temporary, local).await?;
            } else {
                fs::hard_link(&temporary, local)
                    .await
                    .context("creating destination without overwriting")?;
                fs::remove_file(&temporary).await?;
            }
            Ok::<u64, anyhow::Error>(copied)
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&temporary).await;
        }
        result
    })
}

async fn report_progress(
    id: u64,
    bytes: usize,
    progress: &AtomicU64,
    tx: &mpsc::Sender<RuntimeEvent>,
) {
    let total = progress.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64;
    let _ = tx
        .send(RuntimeEvent::TransferProgress { id, bytes: total })
        .await;
}
