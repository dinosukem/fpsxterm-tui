use portable_pty::PtySize;
use ratatui::layout::Rect;
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::{
    config::AppConfig,
    event::Action,
    input::focus::FocusTarget,
    ssh::{HostSource, HostStore, ManagedHost, SshHost},
    terminal::{RuntimeEvent, TerminalManager},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Hosts,
    Terminal,
    Files,
    Transfers,
}

impl Screen {
    pub const ALL: [Self; 4] = [Self::Hosts, Self::Terminal, Self::Files, Self::Transfers];
    pub fn title(self) -> &'static str {
        match self {
            Self::Hosts => "Hosts",
            Self::Terminal => "Terminal",
            Self::Files => "Files",
            Self::Transfers => "Transfers",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePanel {
    Local,
    Remote,
}

pub struct FileManagerState {
    pub local_path: PathBuf,
    pub local_entries: Vec<crate::filesystem::FileEntry>,
    pub selected_local: usize,
    pub remote_path: PathBuf,
    pub remote_entries: Vec<crate::filesystem::FileEntry>,
    pub selected_remote: usize,
    pub remote_host: Option<String>,
    pub active: FilePanel,
    pub loading_local: bool,
    pub loading_remote: bool,
    pub selected_local_paths: HashSet<PathBuf>,
    pub selected_remote_paths: HashSet<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub id: u64,
    pub label: String,
    pub state: String,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    cancel: tokio::sync::watch::Sender<bool>,
}

impl FileManagerState {
    fn new() -> Self {
        Self {
            local_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            local_entries: Vec::new(),
            selected_local: 0,
            remote_path: PathBuf::from("/"),
            remote_entries: Vec::new(),
            selected_remote: 0,
            remote_host: None,
            active: FilePanel::Local,
            loading_local: false,
            loading_remote: false,
            selected_local_paths: HashSet::new(),
            selected_remote_paths: HashSet::new(),
        }
    }
    pub fn entries(&self, side: FilePanel) -> &[crate::filesystem::FileEntry] {
        match side {
            FilePanel::Local => &self.local_entries,
            FilePanel::Remote => &self.remote_entries,
        }
    }
    pub fn selected(&self, side: FilePanel) -> usize {
        match side {
            FilePanel::Local => self.selected_local,
            FilePanel::Remote => self.selected_remote,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FileInputOperation {
    Rename(PathBuf),
    CreateDirectory(PathBuf),
}

#[derive(Debug, Clone)]
pub enum FileModal {
    Input {
        side: FilePanel,
        operation: FileInputOperation,
        value: String,
    },
    ConfirmDelete {
        side: FilePanel,
        entries: Vec<crate::filesystem::FileEntry>,
    },
    ConfirmOverwrite {
        side: FilePanel,
        entry: crate::filesystem::FileEntry,
    },
    PathInput {
        side: FilePanel,
        value: String,
    },
}

#[derive(Debug, Clone)]
pub struct HostDialog {
    pub old_alias: Option<String>,
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: String,
    pub identity_file: String,
    pub field: usize,
}

impl HostDialog {
    fn add() -> Self {
        Self {
            old_alias: None,
            alias: String::new(),
            host_name: String::new(),
            user: String::new(),
            port: "22".into(),
            identity_file: String::new(),
            field: 0,
        }
    }
    fn edit(host: &SshHost) -> Self {
        Self {
            old_alias: Some(host.alias.clone()),
            alias: host.alias.clone(),
            host_name: host.host_name.clone().unwrap_or_default(),
            user: host.user.clone().unwrap_or_default(),
            port: host.port.unwrap_or(22).to_string(),
            identity_file: host
                .identity_file
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            field: 0,
        }
    }
    fn value_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.alias,
            1 => &mut self.host_name,
            2 => &mut self.user,
            3 => &mut self.port,
            _ => &mut self.identity_file,
        }
    }
    fn save(&self) -> anyhow::Result<ManagedHost> {
        Ok(ManagedHost {
            alias: self.alias.clone(),
            host_name: self.host_name.clone(),
            user: self.user.clone(),
            port: self.port.parse()?,
            identity_file: if self.identity_file.is_empty() {
                None
            } else {
                Some(self.identity_file.clone().into())
            },
        })
    }
}

#[derive(Debug, Clone)]
pub enum HostModal {
    Form(HostDialog),
    ConfirmDelete(String),
}

pub struct App {
    pub screen: Screen,
    pub focus: FocusTarget,
    pub config: AppConfig,
    pub should_quit: bool,
    pub status: String,
    pub selected_host: usize,
    pub hosts: Vec<SshHost>,
    pub host_store: HostStore,
    pub host_modal: Option<HostModal>,
    pub terminals: TerminalManager,
    pub runtime_tx: mpsc::Sender<RuntimeEvent>,
    pub terminal_size: PtySize,
    pub terminal_rect: Rect,
    pub files: FileManagerState,
    pub file_modal: Option<FileModal>,
    pub transfers: Vec<TransferRecord>,
    next_transfer_id: u64,
    overwrite_confirmed: bool,
    transfer_semaphore: std::sync::Arc<tokio::sync::Semaphore>,
}

impl App {
    pub fn new(
        config: AppConfig,
        runtime_tx: mpsc::Sender<RuntimeEvent>,
        host_store: HostStore,
        hosts: Vec<SshHost>,
    ) -> Self {
        let terminals = TerminalManager::new(config.terminal.scrollback_lines);
        let transfer_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.transfer.max_concurrent.max(1),
        ));
        Self {
            screen: Screen::Hosts,
            focus: FocusTarget::HostList,
            config,
            should_quit: false,
            status: "Welcome to fpsxterm-tui".into(),
            selected_host: 0,
            hosts,
            host_store,
            host_modal: None,
            terminals,
            runtime_tx,
            terminal_size: PtySize::default(),
            terminal_rect: Rect::default(),
            files: FileManagerState::new(),
            file_modal: None,
            transfers: Vec::new(),
            next_transfer_id: 0,
            overwrite_confirmed: false,
            transfer_semaphore,
        }
    }

    pub fn set_terminal_rect(&mut self, rect: Rect) {
        self.terminal_rect = rect;
    }

    pub fn select_terminal_index(&mut self, index: usize) {
        if let Some((id, _)) = self.terminals.session_titles().get(index) {
            self.terminals.set_active(*id);
        }
    }

    pub fn resize_terminal(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if self.terminal_size.rows == rows && self.terminal_size.cols == cols {
            return;
        }
        self.terminal_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let _ = self.terminals.resize_active(rows, cols);
    }

    pub fn process_runtime_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::FilesLoaded {
                side,
                path,
                entries,
            } => {
                match side {
                    FilePanel::Local => self.files.loading_local = false,
                    FilePanel::Remote => self.files.loading_remote = false,
                }
                match entries {
                    Ok(mut entries) => {
                        if !self.config.ui.show_hidden_files {
                            entries.retain(|entry| !entry.name.starts_with('.'));
                        }
                        match side {
                            FilePanel::Local => {
                                self.files.local_path = path;
                                self.files.local_entries = entries;
                                self.files.selected_local_paths.retain(|path| {
                                    self.files
                                        .local_entries
                                        .iter()
                                        .any(|entry| &entry.path == path)
                                });
                                self.files.selected_local = self
                                    .files
                                    .selected_local
                                    .min(self.files.local_entries.len().saturating_sub(1));
                            }
                            FilePanel::Remote => {
                                self.files.remote_path = path;
                                self.files.remote_entries = entries;
                                self.files.selected_remote_paths.retain(|path| {
                                    self.files
                                        .remote_entries
                                        .iter()
                                        .any(|entry| &entry.path == path)
                                });
                                self.files.selected_remote = self
                                    .files
                                    .selected_remote
                                    .min(self.files.remote_entries.len().saturating_sub(1));
                            }
                        }
                    }
                    Err(error) => {
                        self.status = format!(
                            "Could not read {} directory: {error}",
                            if side == FilePanel::Local {
                                "local"
                            } else {
                                "remote"
                            }
                        )
                    }
                }
            }
            RuntimeEvent::TransferFinished { id, result } => {
                if let Some(transfer) = self.transfers.iter_mut().find(|transfer| transfer.id == id)
                {
                    match result {
                        Ok(bytes) => {
                            transfer.state = "Completed".into();
                            transfer.bytes = bytes;
                            self.status = format!("{} completed ({} bytes)", transfer.label, bytes);
                        }
                        Err(error) => {
                            transfer.state = if error.contains("transfer cancelled") {
                                "Cancelled".into()
                            } else {
                                format!("Failed: {error}")
                            };
                            self.status = format!("Transfer failed: {error}");
                        }
                    }
                }
            }
            RuntimeEvent::TransferProgress { id, bytes } => {
                if let Some(transfer) = self.transfers.iter_mut().find(|transfer| transfer.id == id)
                {
                    transfer.bytes = bytes;
                }
            }
            RuntimeEvent::TransferStarted { id } => {
                if let Some(transfer) = self.transfers.iter_mut().find(|item| item.id == id) {
                    transfer.state = "Running".into();
                }
            }
            RuntimeEvent::FileOperationFinished { side, message } => {
                match side {
                    FilePanel::Local => self.files.loading_local = false,
                    FilePanel::Remote => self.files.loading_remote = false,
                }
                match message {
                    Ok(path) => {
                        match side {
                            FilePanel::Local => self.files.selected_local_paths.clear(),
                            FilePanel::Remote => self.files.selected_remote_paths.clear(),
                        }
                        self.status = format!("File operation completed: {path}");
                        self.refresh_files(side);
                    }
                    Err(error) => self.status = format!("File operation failed: {error}"),
                }
            }
            event => self.terminals.process_event(event),
        }
    }

    fn refresh_files(&mut self, side: FilePanel) {
        let (path, host) = match side {
            FilePanel::Local => (self.files.local_path.clone(), None),
            FilePanel::Remote => {
                let host = self
                    .files
                    .remote_host
                    .clone()
                    .or_else(|| self.hosts.get(self.selected_host).map(|h| h.alias.clone()));
                let Some(host) = host else {
                    self.status = "Select an SSH host before opening remote files".into();
                    return;
                };
                self.files.remote_host = Some(host.clone());
                (self.files.remote_path.clone(), Some(host))
            }
        };
        match side {
            FilePanel::Local => self.files.loading_local = true,
            FilePanel::Remote => self.files.loading_remote = true,
        }
        let tx = self.runtime_tx.clone();
        tokio::spawn(async move {
            let result = if let Some(host) = host {
                match crate::sftp::RemoteFs::connect(&host).await {
                    Ok(mut remote) => {
                        let path = if path == std::path::Path::new("/") {
                            remote.home().await.unwrap_or_else(|_| path.clone())
                        } else {
                            path.clone()
                        };
                        let entries = remote.list(&path).await;
                        let _ = remote.close().await;
                        entries.map(|entries| (path, entries))
                    }
                    Err(error) => Err(error),
                }
            } else {
                crate::filesystem::list_local(&path)
                    .await
                    .map(|entries| (path.clone(), entries))
            };
            let (path, entries) = match result {
                Ok((path, entries)) => (path, Ok(entries)),
                Err(error) => (path, Err(format!("{error:#}"))),
            };
            let _ = tx
                .send(RuntimeEvent::FilesLoaded {
                    side,
                    path,
                    entries,
                })
                .await;
        });
    }

    fn selected_file(&self, side: FilePanel) -> Option<crate::filesystem::FileEntry> {
        let selected = self.files.selected(side);
        let has_parent = match side {
            FilePanel::Local => self.files.local_path.parent().is_some(),
            FilePanel::Remote => self.files.remote_path.parent().is_some(),
        };
        if has_parent && selected == 0 {
            return None;
        }
        self.files
            .entries(side)
            .get(selected.saturating_sub(usize::from(has_parent)))
            .cloned()
    }

    fn run_file_operation(
        &mut self,
        side: FilePanel,
        operation: FileInputOperation,
        value: String,
    ) {
        let host = if side == FilePanel::Remote {
            match self.files.remote_host.clone().or_else(|| {
                self.hosts
                    .get(self.selected_host)
                    .map(|host| host.alias.clone())
            }) {
                Some(host) => Some(host),
                None => {
                    self.status = "Select an SSH host before changing remote files".into();
                    return;
                }
            }
        } else {
            None
        };
        let tx = self.runtime_tx.clone();
        match side {
            FilePanel::Local => self.files.loading_local = true,
            FilePanel::Remote => self.files.loading_remote = true,
        }
        tokio::spawn(async move {
            let result: anyhow::Result<String> = async {
                match operation {
                    FileInputOperation::Rename(path) => {
                        if let Some(host) = host {
                            let mut remote = crate::sftp::RemoteFs::connect(&host).await?;
                            let result = remote.rename(&path, &value).await;
                            let _ = remote.close().await;
                            result.map(|path| path.display().to_string())
                        } else {
                            crate::filesystem::rename_local(&path, &value)
                                .await
                                .map(|path| path.display().to_string())
                        }
                    }
                    FileInputOperation::CreateDirectory(parent) => {
                        if let Some(host) = host {
                            let mut remote = crate::sftp::RemoteFs::connect(&host).await?;
                            let result = remote.create_dir(&parent, &value).await;
                            let _ = remote.close().await;
                            result.map(|_| parent.join(&value).display().to_string())
                        } else {
                            crate::filesystem::create_local_dir(&parent, &value)
                                .await
                                .map(|_| parent.join(&value).display().to_string())
                        }
                    }
                }
            }
            .await;
            let _ = tx
                .send(RuntimeEvent::FileOperationFinished {
                    side,
                    message: result.map_err(|error| format!("{error:#}")),
                })
                .await;
        });
    }

    fn run_delete(&mut self, side: FilePanel, entries: Vec<crate::filesystem::FileEntry>) {
        let host = if side == FilePanel::Remote {
            match self.files.remote_host.clone().or_else(|| {
                self.hosts
                    .get(self.selected_host)
                    .map(|host| host.alias.clone())
            }) {
                Some(host) => Some(host),
                None => {
                    self.status = "Select an SSH host before deleting remote files".into();
                    return;
                }
            }
        } else {
            None
        };
        let tx = self.runtime_tx.clone();
        match side {
            FilePanel::Local => self.files.loading_local = true,
            FilePanel::Remote => self.files.loading_remote = true,
        }
        tokio::spawn(async move {
            let result: anyhow::Result<String> = async {
                if let Some(host) = host {
                    let mut remote = crate::sftp::RemoteFs::connect(&host).await?;
                    let mut deleted = Vec::new();
                    for entry in entries {
                        remote.delete(&entry.path, entry.kind).await?;
                        deleted.push(entry.name);
                    }
                    let _ = remote.close().await;
                    Ok(deleted.join(", "))
                } else {
                    let mut deleted = Vec::new();
                    for entry in entries {
                        crate::filesystem::delete_local(&entry.path).await?;
                        deleted.push(entry.name);
                    }
                    Ok(deleted.join(", "))
                }
            }
            .await;
            let _ = tx
                .send(RuntimeEvent::FileOperationFinished {
                    side,
                    message: result.map_err(|error| format!("{error:#}")),
                })
                .await;
        });
    }

    fn reload_hosts(&mut self) {
        match self.host_store.list() {
            Ok(hosts) => {
                self.hosts = hosts;
                self.selected_host = self.selected_host.min(self.hosts.len().saturating_sub(1));
            }
            Err(error) => self.status = format!("Could not read SSH hosts: {error:#}"),
        }
    }

    fn save_host_dialog(&mut self) {
        let Some(HostModal::Form(dialog)) = self.host_modal.as_ref() else {
            return;
        };
        let old_alias = dialog.old_alias.clone();
        let host = match dialog.save() {
            Ok(host) => host,
            Err(error) => {
                self.status = format!("Invalid host form: {error}");
                return;
            }
        };
        let result = match old_alias.as_deref() {
            Some(old_alias) => self.host_store.update(old_alias, &host),
            None => self.host_store.save(&host),
        };
        match result {
            Ok(()) => {
                self.host_modal = None;
                self.reload_hosts();
                if let Some(index) = self
                    .hosts
                    .iter()
                    .position(|candidate| candidate.alias == host.alias)
                {
                    self.selected_host = index;
                }
                self.status = format!("Saved SSH host {}", host.alias);
            }
            Err(error) => self.status = format!("Could not save SSH host: {error:#}"),
        }
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::NewHost => {
                self.host_modal = Some(HostModal::Form(HostDialog::add()));
            }
            Action::EditHost => match self.hosts.get(self.selected_host) {
                Some(host) if host.source == HostSource::Managed => {
                    self.host_modal = Some(HostModal::Form(HostDialog::edit(host)))
                }
                Some(_) => {
                    self.status =
                        "OpenSSH hosts are read-only; import is required before editing".into()
                }
                None => self.status = "Select a host to edit".into(),
            },
            Action::ImportHost => match self.hosts.get(self.selected_host) {
                Some(host) if host.source == HostSource::OpenSsh => {
                    let alias = host.alias.clone();
                    match self.host_store.import(&alias) {
                        Ok(()) => {
                            self.reload_hosts();
                            self.status =
                                format!("Imported SSH host {alias}; it can now be edited here");
                        }
                        Err(error) => self.status = format!("Could not import SSH host: {error:#}"),
                    }
                }
                Some(_) => {
                    self.status = "This SSH host is already managed by the application".into()
                }
                None => self.status = "Select an SSH host to import".into(),
            },
            Action::DeleteHost => match self.hosts.get(self.selected_host) {
                Some(host) if host.source == HostSource::Managed => {
                    self.host_modal = Some(HostModal::ConfirmDelete(host.alias.clone()))
                }
                Some(_) => self.status = "Hosts from ~/.ssh/config are read-only".into(),
                None => self.status = "Select a managed host to delete".into(),
            },
            Action::ConfirmDelete => {
                if let Some(HostModal::ConfirmDelete(alias)) = self.host_modal.take() {
                    match self.host_store.delete(&alias) {
                        Ok(()) => {
                            self.reload_hosts();
                            self.status = format!("Deleted SSH host {alias}");
                        }
                        Err(error) => self.status = format!("Could not delete SSH host: {error:#}"),
                    }
                }
            }
            Action::CancelDialog => self.host_modal = None,
            Action::NextDialogField(delta) => {
                if let Some(HostModal::Form(dialog)) = self.host_modal.as_mut() {
                    dialog.field = (dialog.field as isize + delta).rem_euclid(5) as usize;
                }
            }
            Action::InsertDialogChar(ch) => {
                if let Some(HostModal::Form(dialog)) = self.host_modal.as_mut()
                    && !ch.is_control()
                {
                    dialog.value_mut().push(ch);
                }
            }
            Action::BackspaceDialog => {
                if let Some(HostModal::Form(dialog)) = self.host_modal.as_mut() {
                    dialog.value_mut().pop();
                }
            }
            Action::SaveDialog => self.save_host_dialog(),
            Action::ReloadHosts => self.reload_hosts(),
            Action::SelectScreen(screen) => {
                self.screen = screen;
                self.focus = match screen {
                    Screen::Hosts => FocusTarget::HostList,
                    Screen::Terminal => FocusTarget::Terminal,
                    Screen::Files => FocusTarget::LocalFiles,
                    Screen::Transfers => FocusTarget::HostList,
                };
                if screen == Screen::Files {
                    let selected_host = self
                        .hosts
                        .get(self.selected_host)
                        .map(|host| host.alias.clone());
                    if selected_host != self.files.remote_host {
                        self.files.remote_path = PathBuf::from("/");
                    }
                    self.files.remote_host = selected_host;
                    self.refresh_files(FilePanel::Local);
                    self.refresh_files(FilePanel::Remote);
                }
            }
            Action::NextScreen => {
                let i = Screen::ALL
                    .iter()
                    .position(|s| *s == self.screen)
                    .unwrap_or(0);
                self.update(Action::SelectScreen(
                    Screen::ALL[(i + 1) % Screen::ALL.len()],
                ));
            }
            Action::PreviousScreen => {
                let i = Screen::ALL
                    .iter()
                    .position(|s| *s == self.screen)
                    .unwrap_or(0);
                self.update(Action::SelectScreen(
                    Screen::ALL[(i + Screen::ALL.len() - 1) % Screen::ALL.len()],
                ));
            }
            Action::MoveSelection(delta) if self.screen == Screen::Files => {
                let side = self.files.active;
                let has_parent = match side {
                    FilePanel::Local => self.files.local_path.parent().is_some(),
                    FilePanel::Remote => self.files.remote_path.parent().is_some(),
                };
                let len = self.files.entries(side).len() + usize::from(has_parent);
                if len > 0 {
                    let selected = match side {
                        FilePanel::Local => &mut self.files.selected_local,
                        FilePanel::Remote => &mut self.files.selected_remote,
                    };
                    *selected = selected.saturating_add_signed(delta).min(len - 1);
                }
            }
            Action::MoveSelection(delta)
                if self.screen == Screen::Hosts && !self.hosts.is_empty() =>
            {
                self.selected_host = self
                    .selected_host
                    .saturating_add_signed(delta)
                    .min(self.hosts.len() - 1);
            }
            Action::MoveSelection(_) => {}
            Action::Focus(target) => self.focus = target,
            Action::ConnectHost => {
                if let Some(alias) = self
                    .hosts
                    .get(self.selected_host)
                    .map(|host| host.alias.clone())
                {
                    match self.terminals.open_remote(
                        &alias,
                        self.terminal_size,
                        self.runtime_tx.clone(),
                    ) {
                        Ok(_) => self.update(Action::SelectScreen(Screen::Terminal)),
                        Err(error) => self.status = format!("Could not start SSH: {error:#}"),
                    }
                } else {
                    self.status = "Select an SSH host first".into();
                }
            }
            Action::OpenLocalTerminal => match self.terminals.open_local(
                &self.config.terminal.shell,
                self.terminal_size,
                self.runtime_tx.clone(),
            ) {
                Ok(_) => self.update(Action::SelectScreen(Screen::Terminal)),
                Err(error) => self.status = format!("Could not start local shell: {error:#}"),
            },
            Action::CloseTerminal => match self.terminals.close_active() {
                Ok(()) => self.status = "Terminal session closed".into(),
                Err(error) => self.status = error.to_string(),
            },
            Action::SendTerminalInput(bytes) => {
                if let Err(error) = self.terminals.write_active(bytes) {
                    self.status = error.to_string();
                }
            }
            Action::SelectTerminal(id) => {
                self.terminals.set_active(id);
            }
            Action::Refresh if self.screen == Screen::Files => {
                self.refresh_files(self.files.active)
            }
            Action::Refresh => self.status = "Refresh is not available on this screen yet".into(),
            Action::FocusFilePanel(side) => {
                self.files.active = side;
                self.focus = match side {
                    FilePanel::Local => FocusTarget::LocalFiles,
                    FilePanel::Remote => FocusTarget::RemoteFiles,
                };
            }
            Action::OpenFileEntry => {
                let side = self.files.active;
                let selected = self.files.selected(side);
                let has_parent = match side {
                    FilePanel::Local => self.files.local_path.parent().is_some(),
                    FilePanel::Remote => self.files.remote_path.parent().is_some(),
                };
                if has_parent && selected == 0 {
                    self.update(Action::ParentDirectory);
                } else if let Some(entry) = self
                    .files
                    .entries(side)
                    .get(selected.saturating_sub(usize::from(has_parent)))
                    .cloned()
                {
                    if entry.kind == crate::filesystem::EntryKind::Directory {
                        match side {
                            FilePanel::Local => self.files.local_path = entry.path,
                            FilePanel::Remote => self.files.remote_path = entry.path,
                        }
                        self.refresh_files(side);
                    } else if entry.kind == crate::filesystem::EntryKind::File {
                        let remote_host = if side == FilePanel::Remote {
                            self.files.remote_host.clone().or_else(|| {
                                self.hosts
                                    .get(self.selected_host)
                                    .map(|host| host.alias.clone())
                            })
                        } else {
                            None
                        };
                        if side == FilePanel::Remote && remote_host.is_none() {
                            self.status = "Select an SSH host before opening a remote file".into();
                            return;
                        }
                        if let Err(error) = self.terminals.open_editor(
                            &self.config.terminal.editor,
                            &entry.path,
                            remote_host.as_deref(),
                            self.terminal_size,
                            self.runtime_tx.clone(),
                        ) {
                            self.status = format!("Could not open editor: {error:#}");
                        } else {
                            self.update(Action::SelectScreen(Screen::Terminal));
                        }
                    } else {
                        self.status =
                            "Symbolic links and special files are not opened automatically".into();
                    }
                }
            }
            Action::ParentDirectory => {
                let side = self.files.active;
                let path = match side {
                    FilePanel::Local => &mut self.files.local_path,
                    FilePanel::Remote => &mut self.files.remote_path,
                };
                if let Some(parent) = path.parent() {
                    *path = parent.to_path_buf();
                    match side {
                        FilePanel::Local => self.files.selected_local = 0,
                        FilePanel::Remote => self.files.selected_remote = 0,
                    }
                    self.refresh_files(side);
                }
            }
            Action::TransferSelected => {
                let side = self.files.active;
                let entry = self.selected_file(side);
                let Some(entry) = entry else {
                    self.status = "Select a file to transfer".into();
                    return;
                };
                if !matches!(
                    entry.kind,
                    crate::filesystem::EntryKind::File | crate::filesystem::EntryKind::Directory
                ) {
                    self.status =
                        "Symbolic links and special files are skipped during transfer".into();
                    return;
                }
                let overwrite = std::mem::take(&mut self.overwrite_confirmed);
                let target_exists = match side {
                    FilePanel::Local => self
                        .files
                        .remote_entries
                        .iter()
                        .any(|item| item.name == entry.name),
                    FilePanel::Remote => self
                        .files
                        .local_entries
                        .iter()
                        .any(|item| item.name == entry.name),
                };
                if entry.kind == crate::filesystem::EntryKind::File && target_exists && !overwrite {
                    self.file_modal = Some(FileModal::ConfirmOverwrite { side, entry });
                    return;
                }
                let Some(host) = self
                    .files
                    .remote_host
                    .clone()
                    .or_else(|| self.hosts.get(self.selected_host).map(|h| h.alias.clone()))
                else {
                    self.status = "Select an SSH host before transferring files".into();
                    return;
                };
                let id = self.next_transfer_id;
                self.next_transfer_id += 1;
                let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
                let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let semaphore = self.transfer_semaphore.clone();
                let destination = match side {
                    FilePanel::Local => self.files.remote_path.join(&entry.name),
                    FilePanel::Remote => self.files.local_path.join(&entry.name),
                };
                let source = entry.path.clone();
                let label = format!(
                    "{} {}",
                    if side == FilePanel::Local {
                        "Upload"
                    } else {
                        "Download"
                    },
                    entry.name
                );
                self.transfers.push(TransferRecord {
                    id,
                    label,
                    state: if semaphore.available_permits() == 0 {
                        "Queued"
                    } else {
                        "Running"
                    }
                    .into(),
                    bytes: 0,
                    total_bytes: (entry.kind == crate::filesystem::EntryKind::File)
                        .then_some(entry.size)
                        .flatten(),
                    cancel,
                });
                self.screen = Screen::Transfers;
                let tx = self.runtime_tx.clone();
                tokio::spawn(async move {
                    let Ok(_permit) = semaphore.acquire_owned().await else {
                        return;
                    };
                    let _ = tx.send(RuntimeEvent::TransferStarted { id }).await;
                    if *cancel_rx.borrow() {
                        let _ = tx
                            .send(RuntimeEvent::TransferFinished {
                                id,
                                result: Err("transfer cancelled".into()),
                            })
                            .await;
                        return;
                    }
                    let result = match crate::sftp::RemoteFs::connect(&host).await {
                        Ok(mut remote) => {
                            let result = if side == FilePanel::Local {
                                crate::transfer::upload_path(
                                    &mut remote,
                                    &source,
                                    &destination,
                                    id,
                                    &tx,
                                    cancel_rx.clone(),
                                    progress.clone(),
                                    overwrite,
                                )
                                .await
                            } else {
                                crate::transfer::download_path(
                                    &mut remote,
                                    &source,
                                    &destination,
                                    id,
                                    &tx,
                                    cancel_rx.clone(),
                                    progress.clone(),
                                    overwrite,
                                )
                                .await
                            };
                            let _ = remote.close().await;
                            result
                        }
                        Err(error) => Err(error),
                    };
                    let result = result.map_err(|error| format!("{error:#}"));
                    let _ = tx.send(RuntimeEvent::TransferFinished { id, result }).await;
                });
            }
            Action::CancelTransfer => {
                if let Some(transfer) = self
                    .transfers
                    .iter()
                    .rev()
                    .find(|item| item.state == "Running")
                {
                    let _ = transfer.cancel.send(true);
                    self.status = format!(
                        "Cancelling {} after the current SFTP request",
                        transfer.label
                    );
                } else {
                    self.status = "There is no running transfer to cancel".into();
                }
            }
            Action::ConfirmFileOverwrite => {
                if let Some(FileModal::ConfirmOverwrite { side, entry }) = self.file_modal.take() {
                    self.files.active = side;
                    let has_parent = match side {
                        FilePanel::Local => self.files.local_path.parent().is_some(),
                        FilePanel::Remote => self.files.remote_path.parent().is_some(),
                    };
                    if let Some(index) = self
                        .files
                        .entries(side)
                        .iter()
                        .position(|item| item.path == entry.path)
                    {
                        match side {
                            FilePanel::Local => {
                                self.files.selected_local = index + usize::from(has_parent)
                            }
                            FilePanel::Remote => {
                                self.files.selected_remote = index + usize::from(has_parent)
                            }
                        }
                        self.overwrite_confirmed = true;
                        self.update(Action::TransferSelected);
                    }
                }
            }
            Action::RenameSelected => {
                let side = self.files.active;
                if let Some(entry) = self.selected_file(side) {
                    self.file_modal = Some(FileModal::Input {
                        side,
                        operation: FileInputOperation::Rename(entry.path),
                        value: entry.name,
                    });
                } else {
                    self.status = "Select a file or folder first".into();
                }
            }
            Action::NewDirectory => {
                let side = self.files.active;
                let parent = match side {
                    FilePanel::Local => self.files.local_path.clone(),
                    FilePanel::Remote => self.files.remote_path.clone(),
                };
                self.file_modal = Some(FileModal::Input {
                    side,
                    operation: FileInputOperation::CreateDirectory(parent),
                    value: String::new(),
                });
            }
            Action::DeleteSelected => {
                let side = self.files.active;
                let selected_paths = match side {
                    FilePanel::Local => &self.files.selected_local_paths,
                    FilePanel::Remote => &self.files.selected_remote_paths,
                };
                let entries = self
                    .files
                    .entries(side)
                    .iter()
                    .filter(|entry| selected_paths.contains(&entry.path))
                    .cloned()
                    .collect::<Vec<_>>();
                let entries = if entries.is_empty() {
                    self.selected_file(side).into_iter().collect()
                } else {
                    entries
                };
                if !entries.is_empty() {
                    self.file_modal = Some(FileModal::ConfirmDelete { side, entries });
                } else {
                    self.status = "Select a file or folder first".into();
                }
            }
            Action::ConfirmFileDelete => {
                if let Some(FileModal::ConfirmDelete { side, entries }) = self.file_modal.take() {
                    self.run_delete(side, entries);
                }
            }
            Action::ToggleFileSelection => {
                let side = self.files.active;
                if let Some(entry) = self.selected_file(side) {
                    let selected = match side {
                        FilePanel::Local => &mut self.files.selected_local_paths,
                        FilePanel::Remote => &mut self.files.selected_remote_paths,
                    };
                    if !selected.remove(&entry.path) {
                        selected.insert(entry.path.clone());
                    }
                    self.status = format!("{} selected item(s)", selected.len());
                }
            }
            Action::SaveFileInput => {
                if let Some(FileModal::Input {
                    side,
                    operation,
                    value,
                }) = self.file_modal.take()
                {
                    self.run_file_operation(side, operation, value);
                }
            }
            Action::CancelFileDialog => self.file_modal = None,
            Action::InsertFileChar(ch) => {
                if !ch.is_control() {
                    match self.file_modal.as_mut() {
                        Some(FileModal::Input { value, .. })
                        | Some(FileModal::PathInput { value, .. }) => value.push(ch),
                        _ => {}
                    }
                }
            }
            Action::BackspaceFileInput => match self.file_modal.as_mut() {
                Some(FileModal::Input { value, .. }) | Some(FileModal::PathInput { value, .. }) => {
                    value.pop();
                }
                _ => {}
            },
            Action::EditCurrentPath => {
                let side = self.files.active;
                let path = match side {
                    FilePanel::Local => &self.files.local_path,
                    FilePanel::Remote => &self.files.remote_path,
                };
                self.file_modal = Some(FileModal::PathInput {
                    side,
                    value: path.display().to_string(),
                });
            }
            Action::ApplyCurrentPath => {
                if let Some(FileModal::PathInput { side, value }) = self.file_modal.take() {
                    let mut path = PathBuf::from(value.trim());
                    if side == FilePanel::Local
                        && (value.trim() == "~" || value.trim().starts_with("~/"))
                        && let Some(home) = std::env::var_os("HOME")
                    {
                        let suffix = value.trim().strip_prefix("~/").unwrap_or("");
                        path = PathBuf::from(home).join(suffix);
                    }
                    if path.is_relative() {
                        let base = match side {
                            FilePanel::Local => &self.files.local_path,
                            FilePanel::Remote => &self.files.remote_path,
                        };
                        path = base.join(path);
                    }
                    match side {
                        FilePanel::Local => self.files.local_path = path,
                        FilePanel::Remote => self.files.remote_path = path,
                    }
                    self.refresh_files(side);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_navigation_wraps_and_preserves_a_single_active_screen() {
        let (tx, _rx) = mpsc::channel(4);
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path().join(".ssh");
        let store = HostStore::new(&ssh_home, ssh_home.join("fpsxterm-tui"));
        let mut app = App::new(AppConfig::default(), tx, store, Vec::new());
        app.update(Action::PreviousScreen);
        assert_eq!(app.screen, Screen::Transfers);
        app.update(Action::NextScreen);
        assert_eq!(app.screen, Screen::Hosts);
    }
}
