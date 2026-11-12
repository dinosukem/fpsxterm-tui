use std::path::{Path, PathBuf};

use super::{EntryKind, FileEntry, safe_child};
use anyhow::{Context, Result, bail};
use tokio::fs;

pub async fn list_local(path: &Path) -> Result<Vec<FileEntry>> {
    let mut dir = fs::read_dir(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let mut entries = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).await?;
        let kind = if metadata.file_type().is_symlink() {
            EntryKind::Symlink
        } else if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            kind,
            size: metadata.is_file().then_some(metadata.len()),
            modified: metadata.modified().ok(),
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Directory)
            .cmp(&(b.kind != EntryKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

pub async fn create_local_dir(parent: &Path, name: &str) -> Result<()> {
    let path = safe_child(parent, name)?;
    fs::create_dir(&path)
        .await
        .with_context(|| format!("creating {}", path.display()))
}

pub async fn rename_local(path: &Path, name: &str) -> Result<PathBuf> {
    let parent = path.parent().context("entry has no parent directory")?;
    let destination = safe_child(parent, name)?;
    if fs::symlink_metadata(&destination).await.is_ok() {
        bail!("destination already exists: {}", destination.display());
    }
    fs::rename(path, &destination)
        .await
        .with_context(|| format!("renaming {}", path.display()))?;
    Ok(destination)
}

pub async fn delete_local(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).await?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).await
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).await
    } else {
        bail!("unsupported filesystem object: {}", path.display())
    }
    .with_context(|| format!("deleting {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_the_selected_directory() {
        assert!(safe_child(Path::new("/tmp"), "../outside").is_err());
        assert!(safe_child(Path::new("/tmp"), "a/b").is_err());
    }

    #[tokio::test]
    async fn listing_marks_symlinks_without_following_them() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        tokio::fs::write(&target, "data").await.unwrap();
        tokio::fs::write(dir.path().join(".hidden"), "hidden")
            .await
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();
        let entries = list_local(dir.path()).await.unwrap();
        assert!(entries.iter().any(|entry| entry.name == ".hidden"));
        #[cfg(unix)]
        assert_eq!(
            entries.iter().find(|e| e.name == "link").unwrap().kind,
            EntryKind::Symlink
        );
    }

    #[tokio::test]
    async fn recursive_delete_removes_link_without_touching_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let tree = dir.path().join("tree");
        tokio::fs::write(&target, "keep").await.unwrap();
        tokio::fs::create_dir(&tree).await.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, tree.join("target-link")).unwrap();
        delete_local(&tree).await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&target).await.unwrap(), "keep");
        assert!(!tree.exists());
    }

    #[tokio::test]
    async fn local_create_rename_and_delete_are_conflict_safe() {
        let dir = tempfile::tempdir().unwrap();
        create_local_dir(dir.path(), "first").await.unwrap();
        let source = dir.path().join("first");
        let destination = rename_local(&source, "renamed").await.unwrap();
        assert!(destination.is_dir());
        tokio::fs::create_dir(dir.path().join("occupied"))
            .await
            .unwrap();
        assert!(rename_local(&destination, "occupied").await.is_err());
        delete_local(&destination).await.unwrap();
        assert!(!destination.exists());
    }
}
