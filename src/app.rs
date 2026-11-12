use {
    anyhow::Context,
    portable_pty::PtySize,
    ratatui::layout::Rect,
    serde::{
        Deserialize,
        Serialize,
    },
    std::{
        collections::{
            HashSet,
            VecDeque,
        },
        path::PathBuf,
        sync::atomic::{
            AtomicU64,
            AtomicUsize,
            Ordering,
        },
        time::{
            Duration,
            Instant,
            SystemTime,
        },
    },
    tokio::sync::mpsc,
};

static NEXT_HOST_FORM_ID: AtomicU64 = AtomicU64::new(1);

use crate::{
    config::{
        AppConfig,
        ConfigStore,
        DockerHostConfig,
        DockerShell,
        HostMetadata,
    },
    event::Action,
    input::focus::FocusTarget,
    ssh::{
        HostSource,
        HostStore,
        ManagedHost,
        SshHost,
    },
    terminal::{
        RuntimeEvent,
        TerminalManager,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Hosts,
    Terminal,
    Files,
    Transfers,
    History,
    Configuration,
}

impl Screen {
    pub const ALL: [Self; 6] = [
        Self::Hosts,
        Self::Terminal,
        Self::Files,
        Self::Transfers,
        Self::History,
        Self::Configuration,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Hosts => "Hosts",
            Self::Terminal => "Terminal",
            Self::Files => "Files",
            Self::Transfers => "Transfers",
            Self::History => "History",
            Self::Configuration => "Configuration",
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
    pub remote_path_initialized: bool,
    pub remote_entries: Vec<crate::filesystem::FileEntry>,
    pub selected_remote: usize,
    pub active: FilePanel,
    pub loading_local: bool,
    pub loading_remote: bool,
    pub selected_local_paths: HashSet<PathBuf>,
    pub selected_remote_paths: HashSet<PathBuf>,
    pub session_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub id: u64,
    pub label: String,
    pub state: String,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    pub source: String,
    pub destination: String,
    pub file_path: PathBuf,
    pub started_at: SystemTime,
    pub elapsed: Instant,
    pub duration: Duration,
    pub terminal_session_id: u64,
    cancel: tokio::sync::watch::Sender<bool>,
}

struct TransferLimiter {
    active: AtomicUsize,
    limit: AtomicUsize,
    notify: tokio::sync::Notify,
}

struct TransferPermit(std::sync::Arc<TransferLimiter>);

impl TransferLimiter {
    fn new(limit: usize) -> Self {
        Self {
            active: AtomicUsize::new(0),
            limit: AtomicUsize::new(limit.max(1)),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn is_full(&self) -> bool {
        self.active.load(Ordering::Acquire) >= self.limit.load(Ordering::Acquire)
    }

    fn set_limit(&self, limit: usize) {
        self.limit.store(limit.max(1), Ordering::Release);
        self.notify.notify_waiters();
        self.notify.notify_one();
    }

    async fn acquire(self: &std::sync::Arc<Self>) -> TransferPermit {
        loop {
            let notified = self.notify.notified();
            let limit = self.limit.load(Ordering::Acquire).max(1);
            if self
                .active
                .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                    (active < limit).then_some(active + 1)
                })
                .is_ok()
            {
                if self.active.load(Ordering::Acquire) < self.limit.load(Ordering::Acquire) {
                    self.notify.notify_one();
                }
                return TransferPermit(self.clone());
            }
            notified.await;
        }
    }
}

impl Drop for TransferPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        self.0.notify.notify_one();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    pub alias: String,
    pub at_unix: u64,
    pub status: String,
    #[serde(skip)]
    pub session_id: Option<u64>,
}

#[derive(Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<HistoryEntry>,
}

impl FileManagerState {
    fn new() -> Self {
        Self {
            local_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            local_entries: Vec::new(),
            selected_local: 0,
            remote_path: PathBuf::from("/"),
            remote_path_initialized: false,
            remote_entries: Vec::new(),
            selected_remote: 0,
            active: FilePanel::Local,
            loading_local: false,
            loading_remote: false,
            selected_local_paths: HashSet::new(),
            selected_remote_paths: HashSet::new(),
            session_id: None,
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
        rename: String,
        rename_touched: bool,
        choice: usize,
    },
    Viewer {
        title: String,
        content: String,
        offset: usize,
    },
    ContextMenu {
        side: FilePanel,
        choice: usize,
    },
    PathInput {
        side: FilePanel,
        value: String,
    },
    CloseSessionTransfers {
        session_id: u64,
        choice: usize,
    },
}

#[derive(Debug, Clone)]
pub struct HostDialog {
    pub form_id: u64,
    pub old_alias: Option<String>,
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: String,
    pub identity_file: String,
    pub field: usize,
    pub revision: u64,
    pub tested_revision: Option<u64>,
    pub testing: bool,
}

#[derive(Debug, Clone)]
pub struct DockerHostDialog {
    pub form_id: u64,
    pub original: Option<DockerHostConfig>,
    pub ssh_host: String,
    pub ssh_hosts: Vec<String>,
    pub container: String,
    pub shell: DockerShell,
    pub containers: Vec<String>,
    pub field: usize,
    pub revision: u64,
    pub tested_revision: Option<u64>,
    pub testing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFeedbackKind {
    Running,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct HostFeedback {
    pub alias: Option<String>,
    pub message: String,
    pub kind: HostFeedbackKind,
}

#[derive(Debug, Clone, Copy)]
pub struct HostTreeRow {
    pub host_index: usize,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
}

impl HostDialog {
    fn add() -> Self {
        Self {
            form_id: NEXT_HOST_FORM_ID.fetch_add(1, Ordering::Relaxed),
            old_alias: None,
            alias: String::new(),
            host_name: String::new(),
            user: String::new(),
            port: "22".into(),
            identity_file: String::new(),
            field: 0,
            revision: 0,
            tested_revision: None,
            testing: false,
        }
    }
    fn edit(host: &SshHost) -> Self {
        Self {
            form_id: NEXT_HOST_FORM_ID.fetch_add(1, Ordering::Relaxed),
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
            revision: 0,
            tested_revision: None,
            testing: false,
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
    DockerForm(DockerHostDialog),
    ConfirmDelete {
        alias: String,
        docker_children: Vec<String>,
        offset: usize,
    },
}

pub struct App {
    pub screen: Screen,
    pub recent_screens: VecDeque<Screen>,
    pub focus: FocusTarget,
    pub config: AppConfig,
    pub configuration_draft: AppConfig,
    pub configuration_field: usize,
    pub configuration_scroll: usize,
    pub configuration_hover: Option<usize>,
    pub configuration_editing: bool,
    pub should_quit: bool,
    pub status: String,
    pub selected_host: usize,
    pub hosts: Vec<SshHost>,
    pub host_store: HostStore,
    pub host_modal: Option<HostModal>,
    pub host_feedback: Option<HostFeedback>,
    pub collapsed_host_groups: HashSet<String>,
    pub hovered_host: Option<usize>,
    pub history: Vec<HistoryEntry>,
    pub selected_history: usize,
    history_path: PathBuf,
    config_store: ConfigStore,
    pub terminals: TerminalManager,
    pub runtime_tx: mpsc::Sender<RuntimeEvent>,
    pub terminal_size: PtySize,
    pub terminal_rect: Rect,
    pub viewport: Rect,
    pub files: FileManagerState,
    pub file_modal: Option<FileModal>,
    pub last_file_click: Option<(Instant, u16, u16)>,
    pub transfers: Vec<TransferRecord>,
    pub selected_transfer: usize,
    pub rename_transfer_target: Option<String>,
    next_transfer_id: u64,
    overwrite_confirmed: bool,
    transfer_limiter: std::sync::Arc<TransferLimiter>,
    pub testing_host: Option<String>,
    pub connection_test_result: Option<(String, Result<String, String>)>,
    local_file_request: u64,
    remote_file_request: u64,
    file_session_paths: std::collections::HashMap<u64, (PathBuf, PathBuf, bool)>,
}

impl App {
    fn report_host(&mut self, alias: Option<String>, kind: HostFeedbackKind, message: String) {
        self.host_feedback = Some(HostFeedback {
            alias,
            message,
            kind,
        });
        self.status.clear();
    }

    pub fn host_tree_rows(&self) -> Vec<HostTreeRow> {
        let mut rows = Vec::new();
        for (index, host) in self.hosts.iter().enumerate() {
            if host.source == HostSource::Docker {
                let parent_exists = host.docker.as_ref().is_some_and(|docker| {
                    self.hosts.iter().any(|candidate| {
                        candidate.source != HostSource::Docker && candidate.alias == docker.ssh_host
                    })
                });
                if !parent_exists {
                    rows.push(HostTreeRow {
                        host_index: index,
                        depth: 0,
                        has_children: false,
                        expanded: false,
                    });
                }
                continue;
            }

            let children: Vec<usize> = self
                .hosts
                .iter()
                .enumerate()
                .filter_map(|(child_index, child)| {
                    child
                        .docker
                        .as_ref()
                        .is_some_and(|docker| docker.ssh_host == host.alias)
                        .then_some(child_index)
                })
                .collect();
            let has_children = !children.is_empty();
            let expanded = has_children && !self.collapsed_host_groups.contains(&host.alias);
            rows.push(HostTreeRow {
                host_index: index,
                depth: 0,
                has_children,
                expanded,
            });
            if expanded {
                rows.extend(children.into_iter().map(|host_index| {
                    HostTreeRow {
                        host_index,
                        depth: 1,
                        has_children: false,
                        expanded: false,
                    }
                }));
            }
        }
        rows
    }

    pub fn move_host_selection(&mut self, delta: isize) {
        let rows = self.host_tree_rows();
        if rows.is_empty() {
            return;
        }
        let position = rows
            .iter()
            .position(|row| row.host_index == self.selected_host)
            .unwrap_or(0);
        let next = position.saturating_add_signed(delta).min(rows.len() - 1);
        self.selected_host = rows[next].host_index;
    }

    pub fn visible_host_row_at(&self, screen_row: u16) -> Option<HostTreeRow> {
        if screen_row < 5 || screen_row >= self.viewport.height.saturating_sub(4) {
            return None;
        }
        let rows = self.host_tree_rows();
        let selected_position = rows
            .iter()
            .position(|row| row.host_index == self.selected_host)
            .unwrap_or(0);
        let visible = usize::from(self.viewport.height.saturating_sub(9)).max(1);
        let offset = selected_position.saturating_add(1).saturating_sub(visible);
        rows.get(usize::from(screen_row - 5).saturating_add(offset))
            .copied()
    }

    pub fn toggle_host_group(&mut self, index: usize) {
        let Some(host) = self.hosts.get(index) else {
            return;
        };
        if host.source == HostSource::Docker {
            if let Some(parent_alias) = host.docker.as_ref().map(|docker| docker.ssh_host.clone())
                && let Some(parent_index) = self.hosts.iter().position(|candidate| {
                    candidate.source != HostSource::Docker && candidate.alias == parent_alias
                })
            {
                self.selected_host = parent_index;
            }
            return;
        }
        if !self.hosts.iter().any(|candidate| {
            candidate
                .docker
                .as_ref()
                .is_some_and(|docker| docker.ssh_host == host.alias)
        }) {
            return;
        }
        if !self.collapsed_host_groups.remove(&host.alias) {
            self.collapsed_host_groups.insert(host.alias.clone());
            self.selected_host = index;
        }
    }

    pub fn expand_host_group(&mut self, index: usize) {
        let Some(host) = self.hosts.get(index) else {
            return;
        };
        if host.source != HostSource::Docker {
            self.collapsed_host_groups.remove(&host.alias);
        }
    }

    pub fn host_metadata(&self, host: &SshHost) -> Option<&HostMetadata> {
        let key = if host.source == HostSource::Docker {
            format!("docker:{}", host.alias)
        } else if host.source == HostSource::Managed {
            format!("ssh:{}", host.alias)
        } else {
            return None;
        };
        self.config.host_metadata.get(&key)
    }

    fn update_host_metadata(&mut self, kind: &str, old_alias: Option<&str>, new_alias: &str) {
        let old_key = old_alias.map(|alias| format!("{kind}:{alias}"));
        let new_key = format!("{kind}:{new_alias}");
        let mut metadata = old_key
            .as_ref()
            .and_then(|key| self.config.host_metadata.remove(key))
            .unwrap_or_default();
        let now = unix_now();
        if old_alias.is_none() {
            metadata.created_at = Some(now);
        }
        metadata.updated_at = Some(now);
        self.config.host_metadata.insert(new_key, metadata);
    }

    fn rename_docker_parent(&mut self, old_alias: &str, new_alias: &str) {
        if old_alias == new_alias {
            return;
        }
        if self.collapsed_host_groups.remove(old_alias) {
            self.collapsed_host_groups.insert(new_alias.to_owned());
        }
        let now = unix_now();
        for docker in &mut self.config.docker_hosts {
            if docker.ssh_host != old_alias {
                continue;
            }
            let old_docker_alias = docker_alias(docker);
            docker.ssh_host = new_alias.to_owned();
            let new_docker_alias = docker_alias(docker);
            let mut metadata = self
                .config
                .host_metadata
                .remove(&format!("docker:{old_docker_alias}"))
                .unwrap_or_default();
            metadata.updated_at = Some(now);
            self.config
                .host_metadata
                .insert(format!("docker:{new_docker_alias}"), metadata);
        }
    }

    fn configuration_keep_field_visible(&mut self) {
        let visible = usize::from(self.viewport.height.saturating_sub(9)).max(1);
        if self.configuration_field < self.configuration_scroll {
            self.configuration_scroll = self.configuration_field;
        } else if self.configuration_field >= self.configuration_scroll + visible {
            self.configuration_scroll = self.configuration_field + 1 - visible;
        }
    }

    pub fn configuration_select(&mut self, field: usize) {
        self.configuration_editing = false;
        self.configuration_field = field.min(8);
        self.configuration_keep_field_visible();
    }

    pub fn configuration_move(&mut self, delta: isize) {
        self.configuration_editing = false;
        self.configuration_field = self.configuration_field.saturating_add_signed(delta).min(8);
        self.configuration_keep_field_visible();
    }

    pub fn configuration_toggle(&mut self) {
        match self.configuration_field {
            1 => {
                self.configuration_draft.ui.mouse_enabled =
                    !self.configuration_draft.ui.mouse_enabled
            }
            2 => {
                self.configuration_draft.ui.show_hidden_files =
                    !self.configuration_draft.ui.show_hidden_files
            }
            3 => {
                self.configuration_draft.files.confirm_delete =
                    !self.configuration_draft.files.confirm_delete
            }
            4 => {
                self.configuration_draft.files.confirm_overwrite =
                    !self.configuration_draft.files.confirm_overwrite
            }
            5..=8 => self.configuration_editing = true,
            _ => {}
        }
    }

    pub fn configuration_insert(&mut self, ch: char) {
        if !self.configuration_editing || ch.is_control() {
            return;
        }
        match self.configuration_field {
            5 => {
                let value = self
                    .configuration_draft
                    .terminal
                    .shell
                    .display()
                    .to_string();
                self.configuration_draft.terminal.shell = PathBuf::from(format!("{value}{ch}"));
            }
            6 => {
                if let Some(editor) = self.configuration_draft.terminal.editor.first_mut() {
                    editor.push(ch);
                } else {
                    self.configuration_draft
                        .terminal
                        .editor
                        .push(ch.to_string());
                }
            }
            7 => {
                self.configuration_draft.terminal.scrollback_lines =
                    append_digit(self.configuration_draft.terminal.scrollback_lines, ch)
            }
            8 => {
                self.configuration_draft.transfer.max_concurrent =
                    append_digit(self.configuration_draft.transfer.max_concurrent, ch)
            }
            _ => {}
        }
    }

    pub fn configuration_backspace(&mut self) {
        if !self.configuration_editing {
            return;
        }
        match self.configuration_field {
            5 => {
                let mut value = self
                    .configuration_draft
                    .terminal
                    .shell
                    .display()
                    .to_string();
                value.pop();
                self.configuration_draft.terminal.shell = PathBuf::from(value);
            }
            6 => {
                if let Some(editor) = self.configuration_draft.terminal.editor.first_mut() {
                    editor.pop();
                }
            }
            7 => self.configuration_draft.terminal.scrollback_lines /= 10,
            8 => self.configuration_draft.transfer.max_concurrent /= 10,
            _ => {}
        }
    }

    pub fn configuration_clear(&mut self) {
        match self.configuration_field {
            5 => self.configuration_draft.terminal.shell = PathBuf::new(),
            6 => self.configuration_draft.terminal.editor.clear(),
            7 => self.configuration_draft.terminal.scrollback_lines = 0,
            8 => self.configuration_draft.transfer.max_concurrent = 0,
            _ => {}
        }
    }

    pub fn save_configuration(&mut self) {
        let mut next = self.configuration_draft.clone();
        next.docker_hosts = self.config.docker_hosts.clone();
        next.host_metadata = self.config.host_metadata.clone();
        if next.terminal.editor.len() == 1 && next.terminal.editor[0].contains(char::is_whitespace)
        {
            next.terminal.editor = next.terminal.editor[0]
                .split_whitespace()
                .map(str::to_owned)
                .collect();
        }
        if next.ui.theme != "dark" {
            self.status = "Only the dark theme is currently supported".into();
            return;
        }
        if next.terminal.shell.as_os_str().is_empty()
            || next.terminal.editor.is_empty()
            || next.terminal.scrollback_lines == 0
            || next.transfer.max_concurrent == 0
        {
            self.status =
                "Shell, editor, scrollback and transfer limit must be non-empty and positive"
                    .into();
            return;
        }
        next.transfer.max_concurrent = next.transfer.max_concurrent.clamp(1, 64);
        next.terminal.scrollback_lines = next.terminal.scrollback_lines.clamp(100, 1_000_000);
        match self
            .config_store
            .save(&next)
            .and_then(|_| self.config_store.load())
        {
            Ok(loaded) => {
                self.config = loaded.clone();
                self.configuration_draft = loaded;
                self.transfer_limiter
                    .set_limit(self.config.transfer.max_concurrent);
                self.terminals
                    .set_scrollback(self.config.terminal.scrollback_lines);
                self.configuration_editing = false;
                if self.screen == Screen::Files {
                    self.refresh_files(self.files.active);
                }
                if self.config.ui.mouse_enabled {
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::event::EnableMouseCapture
                    );
                } else {
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::event::DisableMouseCapture
                    );
                }
                self.status =
                    "Configuration saved and reloaded; terminal changes apply to new sessions"
                        .into();
            }
            Err(error) => self.status = format!("Could not save configuration: {error:#}"),
        }
    }

    pub fn new(
        config: AppConfig,
        runtime_tx: mpsc::Sender<RuntimeEvent>,
        host_store: HostStore,
        mut hosts: Vec<SshHost>,
        history_path: PathBuf,
        config_store: ConfigStore,
    ) -> Self {
        for docker in &config.docker_hosts {
            if valid_docker_host(docker)
                && !hosts.iter().any(|host| host.alias == docker_alias(docker))
            {
                hosts.push(SshHost {
                    alias: docker_alias(docker),
                    host_name: None,
                    user: None,
                    port: None,
                    identity_file: None,
                    source: HostSource::Docker,
                    docker: Some(docker.clone()),
                });
            }
        }
        hosts.sort_by_key(|host| host.alias.to_ascii_lowercase());
        let terminals = TerminalManager::new(config.terminal.scrollback_lines);
        let configuration_draft = config.clone();
        let transfer_limiter =
            std::sync::Arc::new(TransferLimiter::new(config.transfer.max_concurrent));
        Self {
            screen: Screen::Hosts,
            recent_screens: VecDeque::from([Screen::Hosts]),
            focus: FocusTarget::HostList,
            config,
            configuration_draft,
            configuration_field: 0,
            configuration_scroll: 0,
            configuration_hover: None,
            configuration_editing: false,
            should_quit: false,
            status: "Welcome to fpsxterm-tui".into(),
            selected_host: 0,
            hosts,
            host_store,
            host_modal: None,
            host_feedback: None,
            collapsed_host_groups: HashSet::new(),
            hovered_host: None,
            history: read_history(&history_path),
            selected_history: 0,
            history_path,
            config_store,
            terminals,
            runtime_tx,
            terminal_size: PtySize::default(),
            terminal_rect: Rect::default(),
            viewport: Rect::default(),
            files: FileManagerState::new(),
            file_modal: None,
            last_file_click: None,
            transfers: Vec::new(),
            selected_transfer: 0,
            rename_transfer_target: None,
            next_transfer_id: 0,
            overwrite_confirmed: false,
            transfer_limiter,
            testing_host: None,
            connection_test_result: None,
            local_file_request: 0,
            remote_file_request: 0,
            file_session_paths: std::collections::HashMap::new(),
        }
    }

    pub fn set_terminal_rect(&mut self, rect: Rect) {
        self.terminal_rect = rect;
    }
    pub fn set_viewport(&mut self, rect: Rect) {
        self.viewport = rect;
    }

    fn add_history(&mut self, alias: String, status: String, session_id: Option<u64>) {
        let id = self.history.iter().map(|entry| entry.id).max().unwrap_or(0) + 1;
        self.history.insert(
            0,
            HistoryEntry {
                id,
                alias,
                at_unix: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                status,
                session_id,
            },
        );
        self.history.truncate(1000);
        self.selected_history = 0;
        self.save_history();
    }

    fn save_history(&self) {
        let Some(parent) = self.history_path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(data) = toml::to_string_pretty(&HistoryFile {
            entries: self.history.clone(),
        }) else {
            return;
        };
        let Ok(temp) = tempfile::NamedTempFile::new_in(parent) else {
            return;
        };
        if std::fs::write(temp.path(), data).is_ok() {
            let _ = temp.persist(&self.history_path);
        }
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
            RuntimeEvent::ReaderClosed { session_id } => {
                if let Some(item) = self
                    .history
                    .iter_mut()
                    .find(|item| item.session_id == Some(session_id) && item.status == "Started")
                {
                    item.status = "Session ended".into();
                    self.save_history();
                }
                self.terminals
                    .process_event(RuntimeEvent::ReaderClosed { session_id });
            }
            RuntimeEvent::FilesLoaded {
                side,
                session_id,
                request_id,
                path,
                entries,
            } => {
                if self.terminals.active_id() != Some(session_id)
                    || match side {
                        FilePanel::Local => request_id != self.local_file_request,
                        FilePanel::Remote => request_id != self.remote_file_request,
                    }
                {
                    return;
                }
                match side {
                    FilePanel::Local => self.files.loading_local = false,
                    FilePanel::Remote => self.files.loading_remote = false,
                }
                let loaded = entries.is_ok();
                match entries {
                    Ok(mut entries) => {
                        self.files.session_id = Some(session_id);
                        if side == FilePanel::Remote {
                            self.files.remote_path_initialized = true;
                        }
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
                if side == FilePanel::Remote && loaded {
                    self.refresh_files(FilePanel::Local);
                }
            }
            RuntimeEvent::FilePreviewLoaded {
                session_id,
                title,
                content,
            } => {
                if self.terminals.active_id() != Some(session_id) || self.screen != Screen::Files {
                    return;
                }
                match content {
                    Ok(content) => {
                        self.file_modal = Some(FileModal::Viewer {
                            title,
                            content,
                            offset: 0,
                        })
                    }
                    Err(error) => self.status = format!("Could not view remote file: {error}"),
                }
            }
            RuntimeEvent::HostConnectionTested { alias, result } => {
                if self.testing_host.as_deref() == Some(alias.as_str()) {
                    self.testing_host = None;
                    self.connection_test_result = Some((alias.clone(), result.clone()));
                    let (kind, message) = match result {
                        Ok(details) => {
                            (
                                HostFeedbackKind::Success,
                                format!("Test connection succeeded: {details}!"),
                            )
                        }
                        Err(error) => {
                            (
                                HostFeedbackKind::Error,
                                format!("Test connection failed: {error}"),
                            )
                        }
                    };
                    self.report_host(Some(alias), kind, message);
                }
            }
            RuntimeEvent::FormConnectionTested {
                form_id,
                revision,
                result,
            } => {
                let dialog = match self.host_modal.as_mut() {
                    Some(HostModal::Form(form))
                        if form.form_id == form_id && form.revision == revision =>
                    {
                        Some((
                            &mut form.testing,
                            &mut form.tested_revision,
                            form.alias.clone(),
                        ))
                    }
                    Some(HostModal::DockerForm(form))
                        if form.form_id == form_id && form.revision == revision =>
                    {
                        Some((
                            &mut form.testing,
                            &mut form.tested_revision,
                            docker_alias(&DockerHostConfig {
                                ssh_host: form.ssh_host.clone(),
                                container: form.container.clone(),
                                shell: form.shell,
                            }),
                        ))
                    }
                    _ => None,
                };
                if let Some((testing, tested_revision, alias)) = dialog {
                    *testing = false;
                    let (kind, message) = match result {
                        Ok(details) => {
                            *tested_revision = Some(revision);
                            (
                                HostFeedbackKind::Success,
                                format!("Connection succeeded: {details}!"),
                            )
                        }
                        Err(error) => {
                            *tested_revision = None;
                            (
                                HostFeedbackKind::Error,
                                format!("Connection test failed: {error}"),
                            )
                        }
                    };
                    self.report_host(Some(alias), kind, message);
                }
            }
            RuntimeEvent::DockerContainersLoaded {
                form_id,
                host,
                result,
            } => {
                let feedback = if let Some(HostModal::DockerForm(form)) = self.host_modal.as_mut()
                    && form.ssh_host == host
                    && form.form_id == form_id
                {
                    match result {
                        Ok(containers) => {
                            if form.revision == 0
                                && form.container.is_empty()
                                && let Some(first) = containers.first()
                            {
                                form.container = first.clone();
                                form.revision = form.revision.wrapping_add(1);
                            }
                            form.containers = containers;
                            Some((
                                HostFeedbackKind::Success,
                                format!("Loaded {} running containers", form.containers.len()),
                            ))
                        }
                        Err(error) => {
                            Some((
                                HostFeedbackKind::Error,
                                format!("Could not list Docker containers: {error}"),
                            ))
                        }
                    }
                } else {
                    None
                };
                if let Some((kind, message)) = feedback {
                    self.report_host(Some(host), kind, message);
                }
            }
            RuntimeEvent::TransferFinished { id, result } => {
                if let Some(transfer) = self.transfers.iter_mut().find(|transfer| transfer.id == id)
                {
                    match result {
                        Ok(bytes) => {
                            transfer.state = "Completed".into();
                            transfer.bytes = bytes;
                            transfer.duration = transfer.elapsed.elapsed();
                            self.status = format!("{} completed ({} bytes)", transfer.label, bytes);
                        }
                        Err(error) => {
                            transfer.duration = transfer.elapsed.elapsed();
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
                    transfer.started_at = SystemTime::now();
                    transfer.elapsed = Instant::now();
                }
            }
            RuntimeEvent::FileOperationFinished {
                side,
                session_id,
                message,
            } => {
                if self.terminals.active_id() != Some(session_id) {
                    return;
                }
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

    pub fn record_terminal_exit(&mut self, session_id: u64, status: &str) {
        if let Some(item) = self
            .history
            .iter_mut()
            .find(|item| item.session_id == Some(session_id))
        {
            item.status = format!("Exited: {status}");
            self.save_history();
        }
        self.status = format!("Terminal {session_id} exited: {status}");
        if self.screen == Screen::Files && self.files.session_id == Some(session_id) {
            self.update(Action::SelectScreen(Screen::Files));
        }
    }

    fn refresh_files(&mut self, side: FilePanel) {
        let Some(session_id) = self.terminals.active_id() else {
            self.files.local_entries.clear();
            self.files.remote_entries.clear();
            self.files.session_id = None;
            self.status = "Connect or open a terminal session before browsing files".into();
            return;
        };
        let remote = self.terminals.active_remote();
        if remote
            .as_ref()
            .is_some_and(|target| target.container.is_none())
            && side == FilePanel::Local
            && self.files.session_id != Some(session_id)
        {
            self.refresh_files(FilePanel::Remote);
            return;
        }
        if side == FilePanel::Remote && remote.is_none() {
            self.files.remote_entries.clear();
            self.status = "Remote Files require an active SSH terminal session".into();
            return;
        }
        let remote_path_initialized = self.files.remote_path_initialized;
        let (path, remote) = match side {
            FilePanel::Local => (self.files.local_path.clone(), None),
            FilePanel::Remote => (self.files.remote_path.clone(), remote),
        };
        match side {
            FilePanel::Local => {
                self.local_file_request = self.local_file_request.wrapping_add(1);
                self.files.loading_local = true;
            }
            FilePanel::Remote => {
                self.remote_file_request = self.remote_file_request.wrapping_add(1);
                self.files.loading_remote = true;
            }
        }
        let request_id = match side {
            FilePanel::Local => self.local_file_request,
            FilePanel::Remote => self.remote_file_request,
        };
        let tx = self.runtime_tx.clone();
        tokio::spawn(async move {
            let result = async {
                if let Some(remote) = remote {
                    if let Some(container) = remote.container {
                        let remote = crate::docker_fs::DockerFs::new(
                            remote.control_path,
                            remote.ssh_host,
                            container,
                        );
                        let missing = remote.diagnostics().await?;
                        if !missing.is_empty() {
                            let hint = remote
                                .tool_install_hint(&missing)
                                .await?
                                .unwrap_or_default();
                            anyhow::bail!(
                                "container is missing tools: {}. Install suggestion: {hint}",
                                missing.join(", ")
                            );
                        }
                        let resolved_path =
                            if path == std::path::Path::new("/") && !remote_path_initialized {
                                remote.home().await
                            } else {
                                Ok(path.clone())
                            };
                        match resolved_path {
                            Ok(path) => remote.list(&path).await.map(|entries| (path, entries)),
                            Err(error) => Err(error),
                        }
                    } else {
                        match crate::sftp::RemoteFs::connect_control(
                            &remote.control_path,
                            &remote.ssh_host,
                        )
                        .await
                        {
                            Ok(mut remote) => {
                                let resolved_path = if path == std::path::Path::new("/")
                                    && !remote_path_initialized
                                {
                                    remote.home().await
                                } else {
                                    Ok(path.clone())
                                };
                                let entries = match resolved_path {
                                    Ok(path) => {
                                        remote.list(&path).await.map(|entries| (path, entries))
                                    }
                                    Err(error) => Err(error),
                                };
                                let _ = remote.close().await;
                                entries
                            }
                            Err(error) => Err(error),
                        }
                    }
                } else {
                    crate::filesystem::list_local(&path)
                        .await
                        .map(|entries| (path.clone(), entries))
                }
            }
            .await;
            let (path, entries) = match result {
                Ok((path, entries)) => (path, Ok(entries)),
                Err(error) => (path, Err(format!("{error:#}"))),
            };
            let _ = tx
                .send(RuntimeEvent::FilesLoaded {
                    side,
                    session_id,
                    request_id,
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
        let session_id = self.terminals.active_id().unwrap_or(0);
        let remote_target = if side == FilePanel::Remote {
            match self.terminals.active_remote() {
                Some(remote) if self.files.session_id == Some(session_id) => Some(remote),
                _ => {
                    self.status = "Open a remote terminal before changing remote files".into();
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
                        if let Some(target) = &remote_target
                            && let Some(container) = &target.container
                        {
                            let remote = crate::docker_fs::DockerFs::new(
                                target.control_path.clone(),
                                target.ssh_host.clone(),
                                container.clone(),
                            );
                            remote
                                .rename(&path, &value)
                                .await
                                .map(|path| path.display().to_string())
                        } else if let Some(target) = &remote_target {
                            let mut remote = crate::sftp::RemoteFs::connect_control(
                                &target.control_path,
                                &target.ssh_host,
                            )
                            .await?;
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
                        if let Some(target) = &remote_target
                            && let Some(container) = &target.container
                        {
                            let remote = crate::docker_fs::DockerFs::new(
                                target.control_path.clone(),
                                target.ssh_host.clone(),
                                container.clone(),
                            );
                            remote
                                .create_dir(&parent, &value)
                                .await
                                .map(|path| path.display().to_string())
                        } else if let Some(target) = &remote_target {
                            let mut remote = crate::sftp::RemoteFs::connect_control(
                                &target.control_path,
                                &target.ssh_host,
                            )
                            .await?;
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
                    session_id,
                    message: result.map_err(|error| format!("{error:#}")),
                })
                .await;
        });
    }

    fn run_delete(&mut self, side: FilePanel, entries: Vec<crate::filesystem::FileEntry>) {
        let session_id = self.terminals.active_id().unwrap_or(0);
        let remote_target = if side == FilePanel::Remote {
            match self.terminals.active_remote() {
                Some(remote) if self.files.session_id == Some(session_id) => Some(remote),
                _ => {
                    self.status = "Open a remote terminal before deleting remote files".into();
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
                if let Some(target) = remote_target.as_ref()
                    && let Some(container) = &target.container
                {
                    let remote = crate::docker_fs::DockerFs::new(
                        target.control_path.clone(),
                        target.ssh_host.clone(),
                        container.clone(),
                    );
                    let mut deleted = Vec::new();
                    for entry in entries {
                        remote.delete(&entry.path, entry.kind).await?;
                        deleted.push(entry.name);
                    }
                    Ok(deleted.join(", "))
                } else if let Some(target) = remote_target {
                    let mut remote = crate::sftp::RemoteFs::connect_control(
                        &target.control_path,
                        &target.ssh_host,
                    )
                    .await?;
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
                    session_id,
                    message: result.map_err(|error| format!("{error:#}")),
                })
                .await;
        });
    }

    fn reload_hosts(&mut self) {
        match self.host_store.list() {
            Ok(hosts) => {
                self.hosts = hosts;
                for docker in &self.config.docker_hosts {
                    if valid_docker_host(docker)
                        && !self
                            .hosts
                            .iter()
                            .any(|host| host.alias == docker_alias(docker))
                    {
                        self.hosts.push(SshHost {
                            alias: docker_alias(docker),
                            host_name: None,
                            user: None,
                            port: None,
                            identity_file: None,
                            source: HostSource::Docker,
                            docker: Some(docker.clone()),
                        });
                    }
                }
                self.hosts
                    .sort_by_key(|host| host.alias.to_ascii_lowercase());
                self.selected_host = self.selected_host.min(self.hosts.len().saturating_sub(1));
            }
            Err(error) => {
                self.report_host(
                    None,
                    HostFeedbackKind::Error,
                    format!("Could not read SSH hosts: {error:#}"),
                )
            }
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
        let old_host = old_alias.as_ref().and_then(|alias| {
            self.hosts
                .iter()
                .find(|candidate| candidate.alias == *alias)
                .map(|candidate| {
                    ManagedHost {
                        alias: candidate.alias.clone(),
                        host_name: candidate.host_name.clone().unwrap_or_default(),
                        user: candidate.user.clone().unwrap_or_default(),
                        port: candidate.port.unwrap_or(22),
                        identity_file: candidate.identity_file.clone(),
                    }
                })
        });
        if self
            .config
            .docker_hosts
            .iter()
            .any(|docker| docker_alias(docker) == host.alias)
        {
            self.report_host(
                old_alias.clone(),
                HostFeedbackKind::Error,
                format!(
                    "Host alias {} is already used by a Docker child",
                    host.alias
                ),
            );
            return;
        }
        let result = match old_alias.as_deref() {
            Some(old_alias) => self.host_store.update(old_alias, &host),
            None => self.host_store.save(&host),
        };
        match result {
            Ok(()) => {
                let old_config = self.config.clone();
                if let Some(old_alias) = old_alias.as_deref() {
                    self.rename_docker_parent(old_alias, &host.alias);
                }
                self.update_host_metadata("ssh", old_alias.as_deref(), &host.alias);
                if let Err(error) = self.config_store.save(&self.config) {
                    match (old_alias.as_deref(), old_host.as_ref()) {
                        (Some(_), Some(old_host)) => {
                            let _ = self.host_store.update(&host.alias, old_host);
                        }
                        _ => {
                            let _ = self.host_store.delete(&host.alias);
                        }
                    }
                    self.config = old_config;
                    self.report_host(
                        Some(host.alias.clone()),
                        HostFeedbackKind::Error,
                        format!("Could not save SSH host metadata: {error:#}"),
                    );
                    return;
                }
                self.configuration_draft.host_metadata = self.config.host_metadata.clone();
                self.configuration_draft.docker_hosts = self.config.docker_hosts.clone();
                self.host_modal = None;
                self.reload_hosts();
                if let Some(index) = self
                    .hosts
                    .iter()
                    .position(|candidate| candidate.alias == host.alias)
                {
                    self.selected_host = index;
                }
                self.report_host(
                    Some(host.alias.clone()),
                    HostFeedbackKind::Success,
                    format!("Saved SSH host {}", host.alias),
                );
            }
            Err(error) => {
                self.report_host(
                    Some(host.alias.clone()),
                    HostFeedbackKind::Error,
                    format!("Could not save SSH host: {error:#}"),
                )
            }
        }
    }

    fn save_docker_host_dialog(&mut self) {
        let Some(HostModal::DockerForm(form)) = self.host_modal.as_ref() else {
            return;
        };
        let original = form.original.clone();
        let config = DockerHostConfig {
            ssh_host: form.ssh_host.clone(),
            container: form.container.trim().to_owned(),
            shell: form.shell,
        };
        if !valid_docker_host(&config) {
            self.report_host(
                Some(docker_alias(&config)),
                HostFeedbackKind::Error,
                "Container name must use letters, digits, dot, underscore or hyphen".into(),
            );
            return;
        }
        if original.as_ref() != Some(&config)
            && self
                .config
                .docker_hosts
                .iter()
                .any(|existing| existing == &config)
        {
            self.report_host(
                Some(docker_alias(&config)),
                HostFeedbackKind::Error,
                "This Docker container host is already configured".into(),
            );
            return;
        }
        let old_alias = original.as_ref().map(docker_alias);
        if self.hosts.iter().any(|host| {
            host.alias == docker_alias(&config) && Some(host.alias.as_str()) != old_alias.as_deref()
        }) {
            self.report_host(
                Some(docker_alias(&config)),
                HostFeedbackKind::Error,
                "A host with this container@host alias already exists".into(),
            );
            return;
        }
        let old_config = self.config.clone();
        if let Some(original) = &original {
            if let Some(index) = self
                .config
                .docker_hosts
                .iter()
                .position(|current| current == original)
            {
                self.config.docker_hosts[index] = config.clone();
            } else {
                self.report_host(
                    old_alias.clone(),
                    HostFeedbackKind::Error,
                    "Docker host was removed while it was being edited".into(),
                );
                return;
            }
        } else {
            self.config.docker_hosts.push(config.clone());
        }
        let new_alias = docker_alias(&config);
        self.update_host_metadata("docker", old_alias.as_deref(), &new_alias);
        match self.config_store.save(&self.config) {
            Ok(()) => {
                self.configuration_draft.docker_hosts = self.config.docker_hosts.clone();
                self.configuration_draft.host_metadata = self.config.host_metadata.clone();
                self.host_modal = None;
                self.reload_hosts();
                self.selected_host = self
                    .hosts
                    .iter()
                    .position(|host| host.alias == new_alias)
                    .unwrap_or(0);
                self.report_host(
                    Some(new_alias.clone()),
                    HostFeedbackKind::Success,
                    format!("Saved Docker host {new_alias}"),
                );
            }
            Err(error) => {
                self.config = old_config;
                self.report_host(
                    Some(new_alias),
                    HostFeedbackKind::Error,
                    format!("Could not save Docker host: {error:#}"),
                );
            }
        }
    }

    fn start_connection_test(&mut self) {
        let Some(host) = self.hosts.get(self.selected_host).cloned() else {
            self.status = "Select a host to test".into();
            return;
        };
        self.testing_host = Some(host.alias.clone());
        self.connection_test_result = None;
        self.report_host(
            Some(host.alias.clone()),
            HostFeedbackKind::Running,
            "Test connection: running…".into(),
        );
        let tx = self.runtime_tx.clone();
        tokio::spawn(async move {
            let result = test_ssh_connection(&host)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = tx
                .send(RuntimeEvent::HostConnectionTested {
                    alias: host.alias,
                    result,
                })
                .await;
        });
    }

    fn start_form_connection_test(&mut self) {
        let (host, form_id, revision) = match self.host_modal.as_mut() {
            Some(HostModal::Form(form)) => {
                let managed = match form.save() {
                    Ok(host) => host,
                    Err(error) => {
                        self.host_feedback = Some(HostFeedback {
                            alias: Some(form.alias.clone()),
                            message: format!("Invalid host form: {error}"),
                            kind: HostFeedbackKind::Error,
                        });
                        self.status.clear();
                        return;
                    }
                };
                if let Err(error) = crate::ssh::validate_candidate(&managed) {
                    self.host_feedback = Some(HostFeedback {
                        alias: Some(managed.alias.clone()),
                        message: format!("Invalid host form: {error}"),
                        kind: HostFeedbackKind::Error,
                    });
                    self.status.clear();
                    return;
                }
                form.testing = true;
                form.tested_revision = None;
                (
                    SshHost {
                        alias: managed.alias,
                        host_name: Some(managed.host_name),
                        user: Some(managed.user),
                        port: Some(managed.port),
                        identity_file: managed.identity_file,
                        source: HostSource::Managed,
                        docker: None,
                    },
                    form.form_id,
                    form.revision,
                )
            }
            Some(HostModal::DockerForm(form)) => {
                let docker = DockerHostConfig {
                    ssh_host: form.ssh_host.clone(),
                    container: form.container.trim().to_owned(),
                    shell: form.shell,
                };
                if !valid_docker_host(&docker) {
                    self.report_host(
                        Some(docker_alias(&docker)),
                        HostFeedbackKind::Error,
                        "Container name must use letters, digits, dot, underscore or hyphen".into(),
                    );
                    return;
                }
                form.testing = true;
                form.tested_revision = None;
                (
                    SshHost {
                        alias: docker_alias(&docker),
                        host_name: None,
                        user: None,
                        port: None,
                        identity_file: None,
                        source: HostSource::Docker,
                        docker: Some(docker),
                    },
                    form.form_id,
                    form.revision,
                )
            }
            _ => return,
        };
        self.report_host(
            Some(host.alias.clone()),
            HostFeedbackKind::Running,
            format!("Testing {}…", host.alias),
        );
        let tx = self.runtime_tx.clone();
        tokio::spawn(async move {
            let result = test_ssh_connection(&host)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = tx
                .send(RuntimeEvent::FormConnectionTested {
                    form_id,
                    revision,
                    result,
                })
                .await;
        });
    }

    fn start_docker_container_listing(&mut self) {
        let Some(host) = self
            .hosts
            .get(self.selected_host)
            .filter(|host| host.source != HostSource::Docker)
            .map(|host| host.alias.clone())
        else {
            self.status = "Select a remote SSH host first".into();
            return;
        };
        let form_id = NEXT_HOST_FORM_ID.fetch_add(1, Ordering::Relaxed);
        self.host_modal = Some(HostModal::DockerForm(DockerHostDialog {
            form_id,
            original: None,
            ssh_host: host.clone(),
            ssh_hosts: self
                .hosts
                .iter()
                .filter(|candidate| candidate.source != HostSource::Docker)
                .map(|candidate| candidate.alias.clone())
                .collect(),
            container: String::new(),
            shell: DockerShell::Auto,
            containers: Vec::new(),
            field: 1,
            revision: 0,
            tested_revision: None,
            testing: false,
        }));
        self.report_host(
            Some(host.clone()),
            HostFeedbackKind::Running,
            format!("Listing containers on {host}…"),
        );
        let tx = self.runtime_tx.clone();
        tokio::spawn(async move {
            let result = list_running_containers(&host)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = tx
                .send(RuntimeEvent::DockerContainersLoaded {
                    form_id,
                    host,
                    result,
                })
                .await;
        });
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::NewHost => {
                self.host_modal = Some(HostModal::Form(HostDialog::add()));
            }
            Action::NewDockerHost => {
                let selected_parent = self.hosts.get(self.selected_host).and_then(|host| {
                    if host.source != HostSource::Docker {
                        Some(host.alias.clone())
                    } else {
                        host.docker.as_ref().map(|docker| docker.ssh_host.clone())
                    }
                });
                let ssh_hosts: Vec<String> = self
                    .hosts
                    .iter()
                    .filter(|host| host.source != HostSource::Docker)
                    .map(|host| host.alias.clone())
                    .collect();
                if let Some(host) = selected_parent.filter(|alias| ssh_hosts.contains(alias)) {
                    self.host_modal = Some(HostModal::DockerForm(DockerHostDialog {
                        form_id: NEXT_HOST_FORM_ID.fetch_add(1, Ordering::Relaxed),
                        original: None,
                        ssh_host: host,
                        ssh_hosts,
                        container: String::new(),
                        shell: DockerShell::Auto,
                        containers: Vec::new(),
                        field: 0,
                        revision: 0,
                        tested_revision: None,
                        testing: false,
                    }));
                } else {
                    self.status = "Select a remote SSH host before adding a container".into();
                }
            }
            Action::ListDockerContainers => self.start_docker_container_listing(),
            Action::TestConnection => self.start_connection_test(),
            Action::TestDialogConnection => self.start_form_connection_test(),
            Action::EditHost => {
                match self.hosts.get(self.selected_host) {
                    Some(host) if host.source == HostSource::Managed => {
                        self.host_modal = Some(HostModal::Form(HostDialog::edit(host)))
                    }
                    Some(host) if host.source == HostSource::Docker => {
                        let docker = host.docker.clone().expect("Docker host has Docker config");
                        self.host_modal = Some(HostModal::DockerForm(DockerHostDialog {
                            form_id: NEXT_HOST_FORM_ID.fetch_add(1, Ordering::Relaxed),
                            original: Some(docker.clone()),
                            ssh_host: docker.ssh_host.clone(),
                            ssh_hosts: self
                                .hosts
                                .iter()
                                .filter(|candidate| candidate.source != HostSource::Docker)
                                .map(|candidate| candidate.alias.clone())
                                .collect(),
                            container: docker.container,
                            shell: docker.shell,
                            containers: Vec::new(),
                            field: 0,
                            revision: 0,
                            tested_revision: None,
                            testing: false,
                        }));
                    }
                    Some(_) => {
                        let alias = self.hosts[self.selected_host].alias.clone();
                        self.report_host(
                            Some(alias),
                            HostFeedbackKind::Warning,
                            "OpenSSH hosts are read-only; import is required before editing".into(),
                        );
                    }
                    None => self.status = "Select a host to edit".into(),
                }
            }
            Action::ImportHost => {
                match self.hosts.get(self.selected_host) {
                    Some(host) if host.source == HostSource::OpenSsh => {
                        let alias = host.alias.clone();
                        match self.host_store.import(&alias) {
                            Ok(()) => {
                                let previous_config = self.config.clone();
                                self.update_host_metadata("ssh", None, &alias);
                                if let Err(error) = self.config_store.save(&self.config) {
                                    self.config = previous_config;
                                    let _ = self.host_store.delete(&alias);
                                    self.report_host(
                                        Some(alias),
                                        HostFeedbackKind::Error,
                                        format!("Could not save imported host metadata: {error:#}"),
                                    );
                                    return;
                                }
                                self.configuration_draft.host_metadata =
                                    self.config.host_metadata.clone();
                                self.reload_hosts();
                                if let Some(index) =
                                    self.hosts.iter().position(|host| host.alias == alias)
                                {
                                    self.selected_host = index;
                                }
                                self.report_host(
                                    Some(alias.clone()),
                                    HostFeedbackKind::Success,
                                    format!("Imported SSH host {alias}; it can now be edited here"),
                                );
                            }
                            Err(error) => {
                                self.report_host(
                                    Some(alias),
                                    HostFeedbackKind::Error,
                                    format!("Could not import SSH host: {error:#}"),
                                )
                            }
                        }
                    }
                    Some(_) => {
                        self.status = "This SSH host is already managed by the application".into()
                    }
                    None => self.status = "Select an SSH host to import".into(),
                }
            }
            Action::DeleteHost => {
                match self.hosts.get(self.selected_host) {
                    Some(host) if host.source == HostSource::Managed => {
                        let alias = host.alias.clone();
                        let docker_children = self
                            .config
                            .docker_hosts
                            .iter()
                            .filter(|docker| docker.ssh_host == alias)
                            .map(docker_alias)
                            .collect();
                        self.host_modal = Some(HostModal::ConfirmDelete {
                            alias,
                            docker_children,
                            offset: 0,
                        });
                    }
                    Some(host) if host.source == HostSource::Docker => {
                        self.host_modal = Some(HostModal::ConfirmDelete {
                            alias: host.alias.clone(),
                            docker_children: Vec::new(),
                            offset: 0,
                        });
                    }
                    Some(host) => {
                        self.report_host(
                            Some(host.alias.clone()),
                            HostFeedbackKind::Warning,
                            "Hosts from ~/.ssh/config are read-only".into(),
                        )
                    }
                    None => self.status = "Select a managed host to delete".into(),
                }
            }
            Action::ConfirmDelete => {
                let Some(HostModal::ConfirmDelete { alias, .. }) = self.host_modal.clone() else {
                    return;
                };
                let old_config = self.config.clone();
                let docker_index = self
                    .config
                    .docker_hosts
                    .iter()
                    .position(|config| docker_alias(config) == alias);
                let managed_host = self
                    .hosts
                    .iter()
                    .find(|host| host.alias == alias && host.source == HostSource::Managed)
                    .map(|host| {
                        ManagedHost {
                            alias: host.alias.clone(),
                            host_name: host.host_name.clone().unwrap_or_default(),
                            user: host.user.clone().unwrap_or_default(),
                            port: host.port.unwrap_or(22),
                            identity_file: host.identity_file.clone(),
                        }
                    });
                let is_parent = docker_index.is_none();
                let remove_result = if is_parent {
                    self.host_store.delete(&alias)
                } else {
                    Ok(())
                };
                let result = remove_result.and_then(|()| {
                    let removed_aliases: Vec<String> = self
                        .config
                        .docker_hosts
                        .iter()
                        .filter(|docker| {
                            docker_index.is_some_and(|_| docker.ssh_host == alias)
                                || docker_alias(docker) == alias
                        })
                        .map(docker_alias)
                        .collect();
                    self.config
                        .docker_hosts
                        .retain(|docker| docker.ssh_host != alias && docker_alias(docker) != alias);
                    self.config.host_metadata.remove(&format!("ssh:{alias}"));
                    for removed_alias in removed_aliases {
                        self.config
                            .host_metadata
                            .remove(&format!("docker:{removed_alias}"));
                    }
                    self.config_store.save(&self.config)
                });
                match result {
                    Ok(()) => {
                        self.configuration_draft.docker_hosts = self.config.docker_hosts.clone();
                        self.configuration_draft.host_metadata = self.config.host_metadata.clone();
                        self.host_modal = None;
                        self.reload_hosts();
                        self.selected_host =
                            self.selected_host.min(self.hosts.len().saturating_sub(1));
                        self.status =
                            format!("Deleted host {alias} and its configured Docker children");
                    }
                    Err(error) => {
                        if is_parent && let Some(host) = managed_host {
                            let _ = self.host_store.save(&host);
                        }
                        self.config = old_config;
                        self.status = format!("Could not delete host {alias}: {error:#}");
                    }
                }
            }
            Action::ScrollHostDelete(delta) => {
                if let Some(HostModal::ConfirmDelete {
                    docker_children,
                    offset,
                    ..
                }) = self.host_modal.as_mut()
                {
                    *offset = offset
                        .saturating_add_signed(delta)
                        .min(docker_children.len().saturating_sub(1));
                }
            }
            Action::CancelDialog => self.host_modal = None,
            Action::NextDialogField(delta) => {
                if let Some(HostModal::Form(dialog)) = self.host_modal.as_mut() {
                    dialog.field = (dialog.field as isize + delta).rem_euclid(5) as usize;
                } else if let Some(HostModal::DockerForm(dialog)) = self.host_modal.as_mut() {
                    dialog.field = (dialog.field as isize + delta).rem_euclid(3) as usize;
                }
            }
            Action::MoveDockerChoice(delta) => {
                if let Some(HostModal::DockerForm(dialog)) = self.host_modal.as_mut() {
                    if dialog.field == 0 {
                        if dialog.ssh_hosts.is_empty() {
                            return;
                        }
                        let current = dialog
                            .ssh_hosts
                            .iter()
                            .position(|name| name == &dialog.ssh_host)
                            .unwrap_or(0);
                        let next = (current as isize + delta)
                            .rem_euclid(dialog.ssh_hosts.len() as isize)
                            as usize;
                        dialog.ssh_host = dialog.ssh_hosts[next].clone();
                        dialog.containers.clear();
                        dialog.revision = dialog.revision.wrapping_add(1);
                        dialog.tested_revision = None;
                        dialog.testing = false;
                        return;
                    }
                    if dialog.field == 1 {
                        if dialog.containers.is_empty() {
                            return;
                        }
                        let current = dialog
                            .containers
                            .iter()
                            .position(|name| name == &dialog.container)
                            .unwrap_or(0);
                        let next = (current as isize + delta)
                            .rem_euclid(dialog.containers.len() as isize)
                            as usize;
                        dialog.container = dialog.containers[next].clone();
                        dialog.revision = dialog.revision.wrapping_add(1);
                        dialog.tested_revision = None;
                        dialog.testing = false;
                        return;
                    }
                    let index = match dialog.shell {
                        DockerShell::Auto => 0_isize,
                        DockerShell::Bash => 1,
                        DockerShell::Sh => 2,
                    };
                    dialog.shell = match (index + delta).rem_euclid(3) {
                        0 => DockerShell::Auto,
                        1 => DockerShell::Bash,
                        _ => DockerShell::Sh,
                    };
                    dialog.revision = dialog.revision.wrapping_add(1);
                    dialog.tested_revision = None;
                    dialog.testing = false;
                }
            }
            Action::InsertDialogChar(ch) => {
                if let Some(HostModal::Form(dialog)) = self.host_modal.as_mut()
                    && !ch.is_control()
                {
                    dialog.value_mut().push(ch);
                    dialog.revision = dialog.revision.wrapping_add(1);
                    dialog.tested_revision = None;
                    dialog.testing = false;
                } else if let Some(HostModal::DockerForm(dialog)) = self.host_modal.as_mut()
                    && !ch.is_control()
                    && dialog.field == 1
                {
                    dialog.container.push(ch);
                    dialog.revision = dialog.revision.wrapping_add(1);
                    dialog.tested_revision = None;
                    dialog.testing = false;
                }
            }
            Action::BackspaceDialog => {
                if let Some(HostModal::Form(dialog)) = self.host_modal.as_mut() {
                    dialog.value_mut().pop();
                    dialog.revision = dialog.revision.wrapping_add(1);
                    dialog.tested_revision = None;
                    dialog.testing = false;
                } else if let Some(HostModal::DockerForm(dialog)) = self.host_modal.as_mut()
                    && dialog.field == 1
                {
                    dialog.container.pop();
                    dialog.revision = dialog.revision.wrapping_add(1);
                    dialog.tested_revision = None;
                    dialog.testing = false;
                }
            }
            Action::SaveDialog => {
                if matches!(self.host_modal, Some(HostModal::DockerForm(_))) {
                    self.save_docker_host_dialog();
                } else {
                    self.save_host_dialog();
                }
            }
            Action::ReloadHosts => self.reload_hosts(),
            Action::SelectScreen(screen) => {
                let old_screen = self.screen;
                self.screen = screen;
                if old_screen != screen {
                    self.last_file_click = None;
                    if self.recent_screens.back().copied() != Some(screen) {
                        self.recent_screens.push_back(screen);
                        while self.recent_screens.len() > 2 {
                            self.recent_screens.pop_front();
                        }
                    }
                }
                self.focus = match screen {
                    Screen::Hosts => FocusTarget::HostList,
                    Screen::Terminal => FocusTarget::Terminal,
                    Screen::Files => FocusTarget::LocalFiles,
                    Screen::Transfers => FocusTarget::HostList,
                    Screen::History => FocusTarget::HostList,
                    Screen::Configuration => FocusTarget::HostList,
                };
                if screen == Screen::Files {
                    let active_id = self.terminals.active_id();
                    if active_id != self.files.session_id {
                        if let Some(old_id) = self.files.session_id {
                            self.file_session_paths.insert(
                                old_id,
                                (
                                    self.files.local_path.clone(),
                                    self.files.remote_path.clone(),
                                    self.files.remote_path_initialized,
                                ),
                            );
                        }
                        self.files.local_entries.clear();
                        self.files.remote_entries.clear();
                        self.files.session_id = None;
                        self.files.loading_local = false;
                        self.files.loading_remote = false;
                        if let Some(active_id) = active_id {
                            let (local, remote, remote_initialized) = self
                                .file_session_paths
                                .get(&active_id)
                                .cloned()
                                .unwrap_or_else(|| {
                                    (
                                        std::env::current_dir()
                                            .unwrap_or_else(|_| PathBuf::from("/")),
                                        PathBuf::from("/"),
                                        false,
                                    )
                                });
                            self.files.local_path = local;
                            self.files.remote_path = remote;
                            self.files.remote_path_initialized = remote_initialized;
                        }
                    }
                    if active_id.is_some() {
                        let remote = self.terminals.active_remote();
                        self.files.session_id = active_id;
                        if remote.is_some() {
                            self.refresh_files(FilePanel::Remote);
                        } else {
                            self.refresh_files(FilePanel::Local);
                        }
                    } else {
                        self.files.local_entries.clear();
                        self.files.remote_entries.clear();
                        self.status =
                            "Connect or open a terminal session before browsing files".into();
                    }
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
            Action::RecentScreen => {
                if self.recent_screens.len() > 1 {
                    let previous = self.recent_screens.front().copied().unwrap_or(self.screen);
                    self.update(Action::SelectScreen(previous));
                }
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
            Action::MoveSelection(delta) if self.screen == Screen::Hosts => {
                self.move_host_selection(delta);
            }
            Action::MoveSelection(delta)
                if self.screen == Screen::History && !self.history.is_empty() =>
            {
                self.selected_history = self
                    .selected_history
                    .saturating_add_signed(delta)
                    .min(self.history.len() - 1);
            }
            Action::MoveSelection(delta)
                if self.screen == Screen::Transfers && !self.transfers.is_empty() =>
            {
                self.selected_transfer = self
                    .selected_transfer
                    .saturating_add_signed(delta)
                    .min(self.transfers.len() - 1);
            }
            Action::MoveSelection(_) => {}
            Action::Focus(target) => self.focus = target,
            Action::ConnectHost => {
                if let Some(host) = self.hosts.get(self.selected_host).cloned() {
                    let alias = host.alias.clone();
                    let started = if let Some(docker) = host.docker.as_ref() {
                        self.terminals.open_docker(
                            docker,
                            self.terminal_size,
                            self.runtime_tx.clone(),
                        )
                    } else {
                        self.terminals.open_remote(
                            &alias,
                            self.terminal_size,
                            self.runtime_tx.clone(),
                        )
                    };
                    match started {
                        Ok(session_id) => {
                            self.add_history(alias, "Started".into(), Some(session_id));
                            self.report_host(
                                Some(host.alias.clone()),
                                HostFeedbackKind::Success,
                                format!("Connected to {}", host.alias),
                            );
                            self.update(Action::SelectScreen(Screen::Terminal));
                        }
                        Err(error) => {
                            self.add_history(alias.clone(), format!("Failed: {error:#}"), None);
                            self.report_host(
                                Some(alias),
                                HostFeedbackKind::Error,
                                format!("Could not start SSH: {error:#}"),
                            );
                        }
                    }
                } else {
                    self.status = "Select an SSH host first".into();
                }
            }
            Action::ConnectHistory => {
                if let Some(alias) = self
                    .history
                    .get(self.selected_history)
                    .map(|entry| entry.alias.clone())
                {
                    if let Some(index) = self.hosts.iter().position(|host| host.alias == alias) {
                        self.selected_host = index;
                        self.update(Action::ConnectHost);
                    } else {
                        self.status = format!("SSH host '{alias}' is no longer configured");
                    }
                }
            }
            Action::DeleteHistory => {
                if self.selected_history < self.history.len() {
                    self.history.remove(self.selected_history);
                    self.selected_history = self
                        .selected_history
                        .min(self.history.len().saturating_sub(1));
                    self.save_history();
                }
            }
            Action::OpenLocalTerminal => {
                match self.terminals.open_local(
                    &self.config.terminal.shell,
                    self.terminal_size,
                    self.runtime_tx.clone(),
                ) {
                    Ok(_) => self.update(Action::SelectScreen(Screen::Terminal)),
                    Err(error) => self.status = format!("Could not start local shell: {error:#}"),
                }
            }
            Action::CloseTerminal => {
                let active = self.terminals.active_id();
                let running = active.is_some_and(|session_id| {
                    self.transfers.iter().any(|transfer| {
                        transfer.terminal_session_id == session_id
                            && matches!(transfer.state.as_str(), "Running" | "Queued")
                    })
                });
                if let Some(session_id) = active.filter(|_| running) {
                    self.file_modal = Some(FileModal::CloseSessionTransfers {
                        session_id,
                        choice: 1,
                    });
                } else {
                    match self.terminals.close_active() {
                        Ok(()) => self.status = "Terminal session closed".into(),
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            Action::ConfirmSessionClose => {
                let Some(FileModal::CloseSessionTransfers { session_id, choice }) =
                    self.file_modal.take()
                else {
                    return;
                };
                if choice == 0 {
                    for transfer in self.transfers.iter().filter(|transfer| {
                        transfer.terminal_session_id == session_id
                            && matches!(transfer.state.as_str(), "Running" | "Queued")
                    }) {
                        let _ = transfer.cancel.send(true);
                    }
                    if self.terminals.active_id() == Some(session_id) {
                        match self.terminals.close_active() {
                            Ok(()) => {
                                self.status = "Transfers cancelled; terminal session closed".into()
                            }
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                } else {
                    self.status = "Terminal session kept open for active transfers".into();
                }
            }
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
                if self.files.active != side {
                    self.last_file_click = None;
                }
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
                            FilePanel::Remote => {
                                self.files.remote_path = entry.path;
                                self.files.remote_path_initialized = true;
                            }
                        }
                        self.refresh_files(side);
                    } else if entry.kind == crate::filesystem::EntryKind::File {
                        let remote_target = if side == FilePanel::Remote {
                            self.terminals.active_remote()
                        } else {
                            None
                        };
                        if side == FilePanel::Remote
                            && (remote_target.is_none()
                                || self.files.session_id != self.terminals.active_id())
                        {
                            self.status =
                                "Open a remote terminal before opening a remote file".into();
                            return;
                        }
                        let previous_session = self.terminals.active_id();
                        let editor = self.config.terminal.editor.clone();
                        let result = match remote_target.as_ref() {
                            Some(remote) if remote.container.is_some() => {
                                self.terminals.open_docker_editor(
                                    &editor,
                                    &entry.path,
                                    remote,
                                    self.terminal_size,
                                    self.runtime_tx.clone(),
                                )
                            }
                            _ => {
                                self.terminals.open_editor(
                                    &editor,
                                    &entry.path,
                                    remote_target.as_ref(),
                                    self.terminal_size,
                                    self.runtime_tx.clone(),
                                )
                            }
                        };
                        match result {
                            Ok(editor_session) => {
                                let paths = (
                                    self.files.local_path.clone(),
                                    self.files.remote_path.clone(),
                                    self.files.remote_path_initialized,
                                );
                                if let Some(previous_session) = previous_session {
                                    self.file_session_paths
                                        .insert(previous_session, paths.clone());
                                }
                                self.file_session_paths.insert(editor_session, paths);
                                self.files.session_id = Some(editor_session);
                                self.update(Action::SelectScreen(Screen::Terminal));
                            }
                            Err(error) => self.status = format!("Could not open editor: {error:#}"),
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
                    if side == FilePanel::Remote {
                        self.files.remote_path_initialized = true;
                    }
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
                let renamed = self.rename_transfer_target.take();
                let destination_name = renamed.as_deref().unwrap_or(&entry.name);
                let target_exists = match side {
                    FilePanel::Local => {
                        self.files
                            .remote_entries
                            .iter()
                            .any(|item| item.name == destination_name)
                    }
                    FilePanel::Remote => {
                        self.files
                            .local_entries
                            .iter()
                            .any(|item| item.name == destination_name)
                    }
                };
                if target_exists && !overwrite {
                    if !self.config.files.confirm_overwrite {
                        let occupied = match side {
                            FilePanel::Local => &self.files.remote_entries,
                            FilePanel::Remote => &self.files.local_entries,
                        };
                        let mut copy_number = 1usize;
                        let renamed = loop {
                            let candidate = if copy_number == 1 {
                                format!("{} (copy)", entry.name)
                            } else {
                                format!("{} (copy {copy_number})", entry.name)
                            };
                            if !occupied.iter().any(|item| item.name == candidate) {
                                break candidate;
                            }
                            copy_number += 1;
                        };
                        self.rename_transfer_target = Some(renamed);
                        self.update(Action::TransferSelected);
                        return;
                    }
                    self.file_modal = Some(FileModal::ConfirmOverwrite {
                        side,
                        rename: renamed.unwrap_or_else(|| format!("{} (copy)", entry.name)),
                        entry,
                        choice: 0,
                        rename_touched: false,
                    });
                    return;
                }
                let Some(session_id) = self.terminals.active_id() else {
                    self.status = "Open an SSH terminal before transferring files".into();
                    return;
                };
                let Some(remote_target) = self.terminals.active_remote() else {
                    self.status = "Transfers require an active remote terminal session".into();
                    return;
                };
                if self.files.session_id != Some(session_id) {
                    self.status = "Wait for Files to load for the active terminal session".into();
                    return;
                }
                let host = remote_target.ssh_host.clone();
                let remote_label = remote_target
                    .container
                    .as_ref()
                    .map(|container| format!("{container}@{host}"))
                    .unwrap_or_else(|| host.clone());
                let id = self.next_transfer_id;
                self.next_transfer_id += 1;
                let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
                let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let limiter = self.transfer_limiter.clone();
                let target_dir = match side {
                    FilePanel::Local => &self.files.remote_path,
                    FilePanel::Remote => &self.files.local_path,
                };
                let destination = match crate::filesystem::safe_child(target_dir, destination_name)
                {
                    Ok(path) => path,
                    Err(error) => {
                        self.status = format!("Invalid destination name: {error}");
                        return;
                    }
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
                    state: if limiter.is_full() {
                        "Queued"
                    } else {
                        "Running"
                    }
                    .into(),
                    bytes: 0,
                    total_bytes: (entry.kind == crate::filesystem::EntryKind::File)
                        .then_some(entry.size)
                        .flatten(),
                    source: if side == FilePanel::Local {
                        format!("localhost:{}", source.display())
                    } else {
                        format!("{remote_label}:{}", source.display())
                    },
                    destination: if side == FilePanel::Local {
                        format!("{remote_label}:{}", destination.display())
                    } else {
                        format!("localhost:{}", destination.display())
                    },
                    file_path: source.clone(),
                    started_at: SystemTime::now(),
                    elapsed: Instant::now(),
                    duration: Duration::ZERO,
                    terminal_session_id: remote_target.owner_session_id,
                    cancel,
                });
                self.selected_transfer = self.transfers.len().saturating_sub(1);
                self.update(Action::SelectScreen(Screen::Transfers));
                let tx = self.runtime_tx.clone();
                tokio::spawn(async move {
                    let _permit = limiter.acquire().await;
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
                    let result = if let Some(container) = remote_target.container.as_ref() {
                        let remote = crate::docker_fs::DockerFs::new(
                            remote_target.control_path.clone(),
                            host.clone(),
                            container.clone(),
                        );
                        if side == FilePanel::Local {
                            crate::transfer::upload_docker_path(
                                &remote,
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
                            crate::transfer::download_docker_path(
                                &remote,
                                &source,
                                &destination,
                                id,
                                &tx,
                                cancel_rx.clone(),
                                progress.clone(),
                                overwrite,
                            )
                            .await
                        }
                    } else {
                        match crate::sftp::RemoteFs::connect_control(
                            &remote_target.control_path,
                            &host,
                        )
                        .await
                        {
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
                        }
                    };
                    let result = result.map_err(|error| format!("{error:#}"));
                    let _ = tx.send(RuntimeEvent::TransferFinished { id, result }).await;
                });
            }
            Action::CancelTransfer => {
                if let Some(transfer) = self
                    .transfers
                    .get(self.selected_transfer)
                    .filter(|item| item.state == "Running")
                    .or_else(|| {
                        self.transfers
                            .iter()
                            .rev()
                            .find(|item| item.state == "Running")
                    })
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
                if let Some(FileModal::ConfirmOverwrite {
                    side,
                    entry,
                    rename,
                    choice,
                    ..
                }) = self.file_modal.take()
                {
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
                        match choice {
                            0 => {
                                self.rename_transfer_target = Some(rename);
                                self.update(Action::TransferSelected);
                            }
                            1 => {
                                self.overwrite_confirmed = true;
                                self.update(Action::TransferSelected);
                            }
                            _ => {}
                        }
                    }
                }
            }
            Action::OpenFileContextMenu => {
                self.file_modal = Some(FileModal::ContextMenu {
                    side: self.files.active,
                    choice: 0,
                });
            }
            Action::ContextAction(index) => {
                let side = match self.file_modal.take() {
                    Some(FileModal::ContextMenu { side, .. }) => side,
                    _ => return,
                };
                self.files.active = side;
                match index {
                    0 => self.update(Action::OpenFileViewer),
                    1 => self.update(Action::OpenFileEntry),
                    2 => self.update(Action::TransferSelected),
                    3 => self.update(Action::RenameSelected),
                    4 => self.update(Action::NewDirectory),
                    5 => self.update(Action::DeleteSelected),
                    6 => self.update(Action::Refresh),
                    _ => {}
                }
            }
            Action::ChooseFileOption(delta) => {
                match self.file_modal.as_mut() {
                    Some(FileModal::ConfirmOverwrite { choice, .. }) => {
                        *choice = (*choice as isize + delta).rem_euclid(4) as usize
                    }
                    Some(FileModal::ContextMenu { choice, .. }) => {
                        *choice = (*choice as isize + delta).rem_euclid(7) as usize
                    }
                    Some(FileModal::CloseSessionTransfers { choice, .. }) => {
                        *choice = (*choice as isize + delta).rem_euclid(2) as usize
                    }
                    _ => {}
                }
            }
            Action::ScrollViewer(delta) => {
                if let Some(FileModal::Viewer { offset, .. }) = self.file_modal.as_mut() {
                    *offset = offset.saturating_add_signed(delta).min(usize::MAX / 2);
                }
            }
            Action::OpenFileViewer => {
                let side = self.files.active;
                if let Some(entry) = self.selected_file(side) {
                    match entry.kind {
                        crate::filesystem::EntryKind::Directory => {
                            self.update(Action::OpenFileEntry)
                        }
                        crate::filesystem::EntryKind::File => {
                            if side == FilePanel::Local {
                                let result = std::fs::metadata(&entry.path).and_then(|metadata| {
                                    if metadata.len() > 2 * 1024 * 1024 {
                                        return Err(std::io::Error::other("viewer limit is 2 MiB"));
                                    }
                                    std::fs::read_to_string(&entry.path)
                                });
                                match result {
                                    Ok(content) => {
                                        self.file_modal = Some(FileModal::Viewer {
                                            title: entry.path.display().to_string(),
                                            content,
                                            offset: 0,
                                        })
                                    }
                                    Err(error) => {
                                        self.status = format!("Could not read file: {error}")
                                    }
                                }
                            } else {
                                self.status = "Remote viewer is loading".into();
                                let Some(remote_target) = self.terminals.active_remote() else {
                                    self.status =
                                        "Open a remote terminal before viewing remote files".into();
                                    return;
                                };
                                let session_id = self.terminals.active_id().unwrap_or(0);
                                let path = entry.path.clone();
                                let title = entry.path.display().to_string();
                                let tx = self.runtime_tx.clone();
                                tokio::spawn(async move {
                                    let result: anyhow::Result<String> =
                                        if let Some(container) = remote_target.container {
                                            crate::docker_fs::DockerFs::new(
                                                remote_target.control_path,
                                                remote_target.ssh_host,
                                                container,
                                            )
                                            .read_file(&path)
                                            .await
                                        } else {
                                            async {
                                                let mut remote =
                                                    crate::sftp::RemoteFs::connect_control(
                                                        &remote_target.control_path,
                                                        &remote_target.ssh_host,
                                                    )
                                                    .await?;
                                                let data = remote.read_file(&path).await;
                                                let _ = remote.close().await;
                                                data
                                            }
                                            .await
                                        };
                                    let result = result.map_err(|error| format!("{error:#}"));
                                    let _ = tx
                                        .send(RuntimeEvent::FilePreviewLoaded {
                                            session_id,
                                            title,
                                            content: result,
                                        })
                                        .await;
                                });
                            }
                        }
                        _ => self.status = "This entry cannot be viewed".into(),
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
                    if self.config.files.confirm_delete {
                        self.file_modal = Some(FileModal::ConfirmDelete { side, entries });
                    } else {
                        self.run_delete(side, entries);
                    }
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
                        Some(FileModal::ConfirmOverwrite {
                            rename,
                            rename_touched,
                            choice: 0,
                            ..
                        }) => {
                            if !*rename_touched {
                                rename.clear();
                                *rename_touched = true;
                            }
                            rename.push(ch);
                        }
                        _ => {}
                    }
                }
            }
            Action::BackspaceFileInput => {
                match self.file_modal.as_mut() {
                    Some(FileModal::Input { value, .. })
                    | Some(FileModal::PathInput { value, .. }) => {
                        value.pop();
                    }
                    Some(FileModal::ConfirmOverwrite {
                        rename,
                        rename_touched,
                        choice: 0,
                        ..
                    }) => {
                        if !*rename_touched {
                            rename.clear();
                            *rename_touched = true;
                        } else {
                            rename.pop();
                        }
                    }
                    _ => {}
                }
            }
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
            Action::CompletePath => {
                let Some(FileModal::PathInput { side, value }) = self.file_modal.as_mut() else {
                    return;
                };
                let current_path = match side {
                    FilePanel::Local => &self.files.local_path,
                    FilePanel::Remote => &self.files.remote_path,
                };
                let entries = self.files.entries(*side);
                if let Some(completed) = complete_directory_path(value, current_path, entries) {
                    *value = completed;
                }
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
                        FilePanel::Remote => {
                            self.files.remote_path = path;
                            self.files.remote_path_initialized = true;
                        }
                    }
                    self.refresh_files(side);
                }
            }
        }
    }
}

fn read_history(path: &std::path::Path) -> Vec<HistoryEntry> {
    let mut history = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<HistoryFile>(&text).ok())
        .map(|file| file.entries)
        .unwrap_or_default();
    for item in &mut history {
        if item.status == "Started" {
            item.status = "Interrupted (previous run)".into();
        }
    }
    history
}

pub(crate) fn docker_alias(host: &DockerHostConfig) -> String {
    format!("{}@{}", host.container, host.ssh_host)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn valid_docker_host(host: &DockerHostConfig) -> bool {
    valid_docker_name(&host.container) && valid_ssh_alias(&host.ssh_host)
}

fn valid_docker_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_ssh_alias(alias: &str) -> bool {
    !alias.is_empty()
        && !alias.starts_with('-')
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

async fn ssh_command(host: &str, remote_command: &str) -> anyhow::Result<String> {
    if !valid_ssh_alias(host) {
        anyhow::bail!("invalid SSH host alias");
    }
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("ssh")
            .args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "NumberOfPasswordPrompts=0",
                "--",
                host,
                remote_command,
            ])
            .output(),
    )
    .await
    .context("SSH command timed out after 10 seconds")??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                format!("ssh exited with {}", output.status)
            } else {
                stderr
            }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn test_ssh_connection(host: &SshHost) -> anyhow::Result<String> {
    if let Some(docker) = &host.docker {
        if !valid_docker_host(docker) {
            anyhow::bail!("invalid Docker host configuration");
        }
        let inspect = ssh_command(
            &docker.ssh_host,
            &format!(
                "docker inspect --format '{{{{.State.Running}}}}' {}",
                docker.container
            ),
        )
        .await?;
        if inspect != "true" {
            anyhow::bail!("container is not running");
        }
        let shell = match docker.shell {
            DockerShell::Bash => "docker exec {} bash -lc true",
            DockerShell::Sh => "docker exec {} sh -c true",
            DockerShell::Auto => "docker exec {} bash -lc true",
        };
        let check = ssh_command(&docker.ssh_host, &shell.replace("{}", &docker.container)).await;
        if check.is_err() && docker.shell == DockerShell::Auto {
            ssh_command(
                &docker.ssh_host,
                &format!("docker exec {} sh -c true", docker.container),
            )
            .await?;
            return Ok("SSH, Docker access and sh are available".into());
        }
        check?;
        Ok(format!(
            "SSH, Docker access and {:?} are available",
            docker.shell
        ))
    } else {
        if host.source == HostSource::Managed {
            let mut command = tokio::process::Command::new("ssh");
            command.args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "NumberOfPasswordPrompts=0",
                "-p",
            ]);
            command.arg(host.port.unwrap_or(22).to_string());
            if let Some(user) = &host.user {
                command.arg("-l").arg(user);
            }
            if let Some(identity_file) = &host.identity_file {
                command.arg("-i").arg(identity_file);
            }
            command
                .arg("--")
                .arg(host.host_name.as_deref().unwrap_or(&host.alias))
                .arg("true");
            let output = tokio::time::timeout(Duration::from_secs(10), command.output())
                .await
                .context("SSH connection test timed out after 10 seconds")??;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                anyhow::bail!(
                    "{}",
                    if stderr.is_empty() {
                        format!("ssh exited with {}", output.status)
                    } else {
                        stderr
                    }
                );
            }
        } else {
            ssh_command(&host.alias, "true").await?;
        }
        Ok("SSH authentication succeeded".into())
    }
}

async fn list_running_containers(host: &str) -> anyhow::Result<Vec<String>> {
    let output = ssh_command(host, "docker ps --format '{{.Names}}'").await?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|name| valid_docker_name(name))
        .map(str::to_owned)
        .collect())
}

fn append_digit(value: usize, ch: char) -> usize {
    ch.to_digit(10)
        .and_then(|digit| value.checked_mul(10)?.checked_add(digit as usize))
        .unwrap_or(value)
        .min(1_000_000)
}

fn complete_directory_path(
    value: &str,
    current_path: &std::path::Path,
    entries: &[crate::filesystem::FileEntry],
) -> Option<String> {
    let input = value.trim();
    if std::path::Path::new(input) == current_path && !input.ends_with(std::path::MAIN_SEPARATOR) {
        return Some(format!("{value}{}", std::path::MAIN_SEPARATOR));
    }
    let (parent, prefix) = if input.ends_with(std::path::MAIN_SEPARATOR) {
        (std::path::PathBuf::from(input), "")
    } else {
        let path = std::path::Path::new(input);
        (
            path.parent()
                .unwrap_or_else(|| std::path::Path::new(""))
                .to_path_buf(),
            path.file_name()?.to_str()?,
        )
    };
    let parent = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        current_path.join(parent)
    };
    if parent != current_path || prefix.is_empty() {
        return None;
    }

    let matches = entries
        .iter()
        .filter(|entry| {
            entry.kind == crate::filesystem::EntryKind::Directory && entry.name.starts_with(prefix)
        })
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    let first = *matches.first()?;
    let common = matches
        .iter()
        .skip(1)
        .fold(first.to_owned(), |common, name| {
            common
                .chars()
                .zip(name.chars())
                .take_while(|(left, right)| left == right)
                .map(|(ch, _)| ch)
                .collect()
        });
    if common.len() <= prefix.len() {
        return None;
    }
    let mut completed = value.to_owned();
    completed.push_str(&common[prefix.len()..]);
    if matches.len() == 1 {
        completed.push(std::path::MAIN_SEPARATOR);
    }
    Some(completed)
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
        let mut app = App::new(
            AppConfig::default(),
            tx,
            store,
            Vec::new(),
            dir.path().join("history.toml"),
            ConfigStore::from_paths(
                dir.path().join("config.toml"),
                ssh_home.join("fpsxterm-tui"),
                dir.path().join("state"),
            ),
        );
        app.update(Action::PreviousScreen);
        assert_eq!(app.screen, Screen::Configuration);
        app.update(Action::NextScreen);
        assert_eq!(app.screen, Screen::Hosts);
    }

    #[test]
    fn go_to_path_completes_matching_directories_and_adds_separator_for_unique_match() {
        let current = std::path::Path::new("/home/user");
        let entries = [
            crate::filesystem::FileEntry {
                name: "documents".into(),
                path: current.join("documents"),
                kind: crate::filesystem::EntryKind::Directory,
                size: None,
                modified: None,
            },
            crate::filesystem::FileEntry {
                name: "downloads".into(),
                path: current.join("downloads"),
                kind: crate::filesystem::EntryKind::Directory,
                size: None,
                modified: None,
            },
            crate::filesystem::FileEntry {
                name: "draft.txt".into(),
                path: current.join("draft.txt"),
                kind: crate::filesystem::EntryKind::File,
                size: Some(0),
                modified: None,
            },
        ];

        assert_eq!(
            complete_directory_path("/home/user/doc", current, &entries).as_deref(),
            Some("/home/user/documents/")
        );
        assert_eq!(
            complete_directory_path("/home/user/d", current, &entries).as_deref(),
            Some("/home/user/do")
        );
        assert_eq!(
            complete_directory_path("/home/user/draft", current, &entries),
            None
        );
    }

    #[test]
    fn host_tree_groups_docker_children_and_selection_moves_one_visible_row() {
        let (tx, _rx) = mpsc::channel(4);
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path().join(".ssh");
        let docker = DockerHostConfig {
            ssh_host: "production".into(),
            container: "web".into(),
            shell: DockerShell::Auto,
        };
        let mut app = App::new(
            AppConfig {
                docker_hosts: vec![docker],
                ..AppConfig::default()
            },
            tx,
            HostStore::new(&ssh_home, ssh_home.join("fpsxterm-tui")),
            vec![SshHost {
                alias: "production".into(),
                host_name: Some("prod.example".into()),
                user: Some("ubuntu".into()),
                port: Some(22),
                identity_file: None,
                source: HostSource::Managed,
                docker: None,
            }],
            dir.path().join("history.toml"),
            ConfigStore::from_paths(
                dir.path().join("config.toml"),
                ssh_home.join("fpsxterm-tui"),
                dir.path().join("state"),
            ),
        );
        app.hosts.push(SshHost {
            alias: "staging".into(),
            host_name: Some("stage.example".into()),
            user: Some("ubuntu".into()),
            port: Some(22),
            identity_file: None,
            source: HostSource::Managed,
            docker: None,
        });
        app.hosts
            .sort_by_key(|host| host.alias.to_ascii_lowercase());

        let rows = app.host_tree_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].depth, 0);
        assert!(rows[0].has_children);
        assert_eq!(rows[1].depth, 1);
        app.selected_host = rows[0].host_index;
        app.move_host_selection(1);
        assert_eq!(app.hosts[app.selected_host].source, HostSource::Docker);

        app.toggle_host_group(rows[0].host_index);
        assert_eq!(app.host_tree_rows().len(), 2);
        assert_eq!(app.hosts[app.selected_host].alias, "production");

        app.update_host_metadata("ssh", Some("production"), "production-renamed");
        let metadata = app
            .config
            .host_metadata
            .get("ssh:production-renamed")
            .unwrap();
        assert_eq!(metadata.created_at, None);
        assert!(metadata.updated_at.is_some());
    }

    #[test]
    fn configuration_selection_scrolls_the_selected_field_into_view() {
        let (tx, _rx) = mpsc::channel(4);
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path().join(".ssh");
        let mut app = App::new(
            AppConfig::default(),
            tx,
            HostStore::new(&ssh_home, ssh_home.join("fpsxterm-tui")),
            Vec::new(),
            dir.path().join("history.toml"),
            ConfigStore::from_paths(
                dir.path().join("config.toml"),
                ssh_home.join("fpsxterm-tui"),
                dir.path().join("state"),
            ),
        );
        app.viewport = Rect::new(0, 0, 80, 15);

        app.configuration_select(8);

        assert_eq!(app.configuration_field, 8);
        assert!(app.configuration_scroll > 0);
    }
}
