use {
    super::temporary_path,
    crate::{
        sftp::RemoteFs,
        terminal::RuntimeEvent,
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
        time::Duration,
    },
    tokio::{
        fs,
        io::AsyncReadExt,
        sync::{
            mpsc,
            watch,
        },
    },
};

const SFTP_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

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
    let mut output = tokio::time::timeout(
        SFTP_IDLE_TIMEOUT,
        options.write(true).create_new(true).open(&temporary),
    )
    .await
    .context("timed out creating remote temporary file")?
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
            tokio::time::timeout(SFTP_IDLE_TIMEOUT, output.write_all(&buffer[..count]))
                .await
                .context("timed out writing remote file")??;
            copied += count as u64;
            let total = progress.fetch_add(count as u64, Ordering::Relaxed) + count as u64;
            let _ = tx
                .send(RuntimeEvent::TransferProgress { id, bytes: total })
                .await;
        }
        tokio::time::timeout(SFTP_IDLE_TIMEOUT, output.close())
            .await
            .context("timed out closing remote file")??;
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
