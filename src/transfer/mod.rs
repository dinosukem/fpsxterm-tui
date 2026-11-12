use std::{
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

mod docker;
mod download;
mod upload;

pub use {
    docker::{
        download_path as download_docker_path,
        upload_path as upload_docker_path,
    },
    download::download_path,
    upload::upload_path,
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_path(destination: &Path) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    destination.with_file_name(format!(".{name}.fpsxterm-{id}.part"))
}
