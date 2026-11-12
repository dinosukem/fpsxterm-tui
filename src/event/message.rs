use crate::{app::FilePanel, filesystem::FileEntry};

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
        path: std::path::PathBuf,
        entries: Result<Vec<FileEntry>, String>,
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
        message: Result<String, String>,
    },
    FilePreviewLoaded {
        title: String,
        content: Result<String, String>,
    },
}
