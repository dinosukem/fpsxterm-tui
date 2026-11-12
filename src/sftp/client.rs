use {
    crate::filesystem::{
        EntryKind,
        FileEntry,
        safe_child,
    },
    anyhow::{
        Context,
        Result,
        bail,
    },
    futures_util::TryStreamExt,
    openssh::{
        KnownHosts,
        Session,
    },
    openssh_sftp_client::{
        Sftp,
        SftpOptions,
    },
    std::{
        future::Future,
        path::{
            Path,
            PathBuf,
        },
        time::{
            Duration,
            UNIX_EPOCH,
        },
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn request<T, E>(future: impl Future<Output = std::result::Result<T, E>>) -> Result<T>
where
    E: std::error::Error + Send + Sync + 'static,
{
    tokio::time::timeout(REQUEST_TIMEOUT, future)
        .await
        .context("SFTP request timed out")?
        .map_err(Into::into)
}

pub struct RemoteFs {
    sftp: Sftp,
}

impl RemoteFs {
    pub fn client(&self) -> &Sftp {
        &self.sftp
    }

    pub async fn connect(alias: &str) -> Result<Self> {
        let session = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Session::connect(alias, KnownHosts::Strict),
        )
        .await
        .context("timed out connecting to SSH host")?
        .with_context(|| {
            format!(
                "connecting to SSH host {alias} (SFTP uses OpenSSH config, agent and known_hosts)"
            )
        })?;
        let sftp = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Sftp::from_session(session, SftpOptions::default()),
        )
        .await
        .context("timed out starting SFTP subsystem")?
        .with_context(|| format!("starting SFTP subsystem for {alias}"))?;
        Ok(Self { sftp })
    }

    pub async fn connect_control(control_path: &Path, alias: &str) -> Result<Self> {
        let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
        while !control_path.exists() {
            if tokio::time::Instant::now() >= deadline {
                bail!("SSH terminal session for {alias} is not connected yet");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let session = std::sync::Arc::new(Session::resume(
            control_path.to_path_buf().into_boxed_path(),
            None,
        ));
        let sftp = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Sftp::from_clonable_session(session, SftpOptions::default()),
        )
        .await
        .context("timed out starting SFTP on the active terminal SSH session")?
        .with_context(|| format!("starting SFTP subsystem for active session {alias}"))?;
        Ok(Self { sftp })
    }

    pub async fn home(&mut self) -> Result<PathBuf> {
        let mut fs = self.sftp.fs();
        request(fs.canonicalize("."))
            .await
            .context("resolving remote home directory")
    }

    pub async fn list(&mut self, path: &Path) -> Result<Vec<FileEntry>> {
        let mut fs = self.sftp.fs();
        let dir = request(fs.open_dir(path))
            .await
            .with_context(|| format!("opening remote directory {}", path.display()))?;
        let mut entries = Vec::new();
        let mut stream = Box::pin(dir.read_dir());
        while let Some(entry) = request(stream.try_next()).await? {
            let metadata = entry.metadata();
            let kind = match metadata.file_type() {
                Some(kind) if kind.is_dir() => EntryKind::Directory,
                Some(kind) if kind.is_file() => EntryKind::File,
                Some(kind) if kind.is_symlink() => EntryKind::Symlink,
                _ => EntryKind::Other,
            };
            let name = entry.filename().to_string_lossy().into_owned();
            if name == "." || name == ".." {
                continue;
            }
            let entry_path = path.join(&name);
            entries.push(FileEntry {
                name,
                path: entry_path,
                kind,
                size: metadata.len(),
                modified: metadata.modified().and_then(|time| {
                    let seconds = time.as_duration().as_secs();
                    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
                }),
            });
        }
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Directory)
                .cmp(&(b.kind != EntryKind::Directory))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub async fn read_file(&mut self, path: &Path) -> Result<String> {
        let mut input = request(self.sftp.open(path))
            .await
            .context("opening remote file")?;
        let mut bytes = Vec::new();
        while let Some(chunk) = request(input.read(64 * 1024, bytes::BytesMut::new())).await? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                request(input.close()).await?;
                bail!("viewer limit is 2 MiB");
            }
            bytes.extend_from_slice(&chunk);
        }
        request(input.close()).await?;
        String::from_utf8(bytes).context("file is not valid UTF-8 text")
    }

    pub async fn create_dir(&mut self, parent: &Path, name: &str) -> Result<()> {
        let path = safe_child(parent, name)?;
        request(self.sftp.fs().create_dir(path))
            .await
            .context("creating remote directory")
    }

    pub async fn ensure_dir(&mut self, path: &Path) -> Result<()> {
        let mut fs = self.sftp.fs();
        if let Ok(metadata) = request(fs.symlink_metadata(path)).await {
            if metadata.file_type().is_some_and(|kind| kind.is_dir()) {
                return Ok(());
            }
            bail!(
                "destination exists and is not a directory: {}",
                path.display()
            );
        }
        let parent = path.parent().context("remote target has no parent")?;
        let name = path
            .file_name()
            .context("remote target has no name")?
            .to_string_lossy();
        self.create_dir(parent, &name).await
    }

    pub async fn rename(&mut self, path: &Path, name: &str) -> Result<PathBuf> {
        let parent = path
            .parent()
            .context("remote entry has no parent directory")?;
        let destination = safe_child(parent, name)?;
        self.rename_path(path, &destination).await?;
        Ok(destination)
    }

    pub async fn rename_path(&mut self, path: &Path, destination: &Path) -> Result<()> {
        let mut fs = self.sftp.fs();
        if request(fs.symlink_metadata(&destination)).await.is_ok() {
            bail!("destination already exists: {}", destination.display());
        }
        request(fs.rename(path, &destination))
            .await
            .context("renaming remote entry")?;
        Ok(())
    }

    pub async fn replace_path(&mut self, path: &Path, destination: &Path) -> Result<()> {
        request(self.sftp.fs().rename(path, destination))
            .await
            .context("replacing remote destination")
    }

    pub async fn delete(&mut self, path: &Path, kind: EntryKind) -> Result<()> {
        if kind == EntryKind::Directory {
            let mut pending = vec![path.to_path_buf()];
            let mut directories = Vec::new();
            while let Some(directory) = pending.pop() {
                directories.push(directory.clone());
                for entry in self.list(&directory).await? {
                    match entry.kind {
                        EntryKind::Directory => pending.push(entry.path),
                        _ => {
                            request(self.sftp.fs().remove_file(&entry.path))
                                .await
                                .context("deleting remote file or symbolic link")?
                        }
                    }
                }
            }
            let mut fs = self.sftp.fs();
            for directory in directories.into_iter().rev() {
                request(fs.remove_dir(directory))
                    .await
                    .context("deleting remote directory")?;
            }
            Ok(())
        } else {
            request(self.sftp.fs().remove_file(path))
                .await
                .context("deleting remote file or symbolic link")
        }
    }

    pub async fn close(self) -> Result<()> {
        request(self.sftp.close())
            .await
            .context("closing SFTP connection")
    }
}
