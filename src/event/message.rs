use crate::{
    app::FilePanel,
    filesystem::FileEntry,
};

#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    Output {
        session_id: u64,
        bytes: Vec<u8>,
    },
    ReaderClosed {
        session_id: u64,
    },
    FilesLoaded {
        side: FilePanel,
        session_id: u64,
        request_id: u64,
        path: std::path::PathBuf,
        entries: Result<Vec<FileEntry>, String>,
    },
    HostConnectionTested {
        alias: String,
        result: Result<String, String>,
    },
    FormConnectionTested {
        form_id: u64,
        revision: u64,
        result: Result<String, String>,
    },
    DockerContainersLoaded {
        form_id: u64,
        host: String,
        result: Result<Vec<String>, String>,
    },
    TransferFinished {
        id: u64,
        result: Result<u64, String>,
    },
    TransferProgress {
        id: u64,
        bytes: u64,
    },
    TransferStarted {
        id: u64,
    },
    FileOperationFinished {
        side: FilePanel,
        session_id: u64,
        message: Result<String, String>,
    },
    FilePreviewLoaded {
        session_id: u64,
        title: String,
        content: Result<String, String>,
    },
}
