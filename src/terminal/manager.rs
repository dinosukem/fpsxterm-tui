use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self as std_mpsc, SyncSender},
    thread,
};

pub use crate::event::RuntimeEvent;
use anyhow::{Context, Result, anyhow};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;
use vt100::{MouseProtocolEncoding, MouseProtocolMode, Parser};

struct TerminalSession {
    id: u64,
    title: String,
    parser: Parser,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    input: SyncSender<Vec<u8>>,
    reader_closed: bool,
    exited: bool,
}

#[derive(Default)]
pub struct TerminalManager {
    sessions: BTreeMap<u64, TerminalSession>,
    active: Option<u64>,
    next_id: u64,
    scrollback: usize,
}

impl TerminalManager {
    pub fn new(scrollback: usize) -> Self {
        Self {
            sessions: BTreeMap::new(),
            active: None,
            next_id: 0,
            scrollback,
        }
    }

    pub fn active_screen(&self) -> Option<&vt100::Screen> {
        self.active
            .and_then(|id| self.sessions.get(&id))
            .map(|s| s.parser.screen())
    }

    pub fn active_title(&self) -> Option<&str> {
        self.active
            .and_then(|id| self.sessions.get(&id))
            .map(|s| s.title.as_str())
    }

    pub fn session_titles(&self) -> Vec<(u64, String)> {
        self.sessions
            .values()
            .map(|s| (s.id, s.title.clone()))
            .collect()
    }

    pub fn active_id(&self) -> Option<u64> {
        self.active
    }

    pub fn active_mouse_mode(&self) -> MouseProtocolMode {
        self.active_screen()
            .map(|screen| screen.mouse_protocol_mode())
            .unwrap_or(MouseProtocolMode::None)
    }

    pub fn active_mouse_encoding(&self) -> MouseProtocolEncoding {
        self.active_screen()
            .map(|screen| screen.mouse_protocol_encoding())
            .unwrap_or_default()
    }

    pub fn active_application_cursor(&self) -> bool {
        self.active_screen()
            .is_some_and(|screen| screen.application_cursor())
    }

    pub fn active_bracketed_paste(&self) -> bool {
        self.active_screen()
            .is_some_and(|screen| screen.bracketed_paste())
    }

    pub fn set_active(&mut self, id: u64) -> bool {
        if self.sessions.contains_key(&id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    pub fn open_local(
        &mut self,
        shell: &Path,
        size: PtySize,
        output: mpsc::Sender<RuntimeEvent>,
    ) -> Result<u64> {
        let mut command = CommandBuilder::new(shell);
        command.arg("-l");
        command.env("TERM", "xterm-256color");
        self.open(
            command,
            shell
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("local")
                .to_owned(),
            size,
            output,
        )
    }

    pub fn open_remote(
        &mut self,
        alias: &str,
        size: PtySize,
        output: mpsc::Sender<RuntimeEvent>,
    ) -> Result<u64> {
        let mut command = CommandBuilder::new("ssh");
        command.args(["-tt", "--", alias]);
        command.env("TERM", "xterm-256color");
        self.open(command, alias.to_owned(), size, output)
    }

    pub fn open_editor(
        &mut self,
        editor: &[String],
        path: &Path,
        remote: Option<&str>,
        size: PtySize,
        output: mpsc::Sender<RuntimeEvent>,
    ) -> Result<u64> {
        let Some(program) = editor.first() else {
            return Err(anyhow!("terminal.editor must contain an executable"));
        };
        let (command, title) = match remote {
            None => {
                let mut command = CommandBuilder::new(program);
                command.args(&editor[1..]);
                command.arg("--");
                command.arg(path);
                command.env("TERM", "xterm-256color");
                (command, format!("{} — {}", program, path.display()))
            }
            Some(alias) => {
                let mut args = editor
                    .iter()
                    .map(|arg| shell_quote(arg))
                    .collect::<Vec<_>>();
                args.push("--".into());
                args.push(shell_quote(&path.to_string_lossy()));
                let remote_command = format!("exec {}", args.join(" "));
                let mut command = CommandBuilder::new("ssh");
                command.args(["-tt", "--", alias]);
                command.arg(remote_command);
                command.env("TERM", "xterm-256color");
                (command, format!("{}:{}", alias, path.display()))
            }
        };
        self.open(command, title, size, output)
    }

    fn open(
        &mut self,
        command: CommandBuilder,
        title: String,
        size: PtySize,
        output: mpsc::Sender<RuntimeEvent>,
    ) -> Result<u64> {
        let pty = native_pty_system().openpty(size)?;
        let child = pty
            .slave
            .spawn_command(command)
            .context("starting process in PTY")?;
        drop(pty.slave);
        let reader = pty
            .master
            .try_clone_reader()
            .context("opening PTY output stream")?;
        let writer = pty
            .master
            .take_writer()
            .context("opening PTY input stream")?;
        let id = self.next_id.wrapping_add(1).max(1);
        self.next_id = id;
        let (input, input_rx) = std_mpsc::sync_channel::<Vec<u8>>(16);
        thread::Builder::new()
            .name(format!("fpsxterm-pty-writer-{id}"))
            .spawn(move || writer_loop(writer, input_rx))
            .context("starting PTY input worker")?;
        let output_id = id;
        thread::Builder::new()
            .name(format!("fpsxterm-pty-reader-{id}"))
            .spawn(move || reader_loop(reader, output_id, output))
            .context("starting PTY output worker")?;
        self.sessions.insert(
            id,
            TerminalSession {
                id,
                title,
                parser: Parser::new(size.rows.max(1), size.cols.max(1), self.scrollback),
                child,
                master: pty.master,
                input,
                reader_closed: false,
                exited: false,
            },
        );
        self.active = Some(id);
        Ok(id)
    }

    pub fn write_active(&self, bytes: Vec<u8>) -> Result<()> {
        let id = self
            .active
            .ok_or_else(|| anyhow!("no active terminal session"))?;
        self.sessions
            .get(&id)
            .ok_or_else(|| anyhow!("active terminal session no longer exists"))?
            .input
            .send(bytes)
            .map_err(|_| anyhow!("terminal input worker has stopped"))
    }

    pub fn resize_active(&mut self, rows: u16, cols: u16) -> Result<()> {
        let Some(id) = self.active else { return Ok(()) };
        let Some(session) = self.sessions.get_mut(&id) else {
            return Ok(());
        };
        let size = PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        session.master.resize(size)?;
        session.parser.screen_mut().set_size(size.rows, size.cols);
        Ok(())
    }

    pub fn process_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Output { session_id, bytes } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.parser.process(&bytes);
                }
            }
            RuntimeEvent::ReaderClosed { session_id } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.reader_closed = true;
                }
            }
            RuntimeEvent::FilesLoaded { .. } => {}
            RuntimeEvent::TransferFinished { .. } => {}
            RuntimeEvent::TransferProgress { .. } => {}
            RuntimeEvent::TransferStarted { .. } => {}
            RuntimeEvent::FileOperationFinished { .. } => {}
            RuntimeEvent::FilePreviewLoaded { .. } => {}
        }
    }

    pub fn poll_exits(&mut self) -> Vec<(u64, String)> {
        let mut exited = Vec::new();
        for session in self.sessions.values_mut() {
            if session.exited {
                continue;
            }
            match session.child.try_wait() {
                Ok(Some(status)) => {
                    session.exited = true;
                    exited.push((session.id, status.to_string()));
                }
                Ok(None) => {}
                Err(error) => {
                    session.exited = true;
                    exited.push((session.id, format!("process status error: {error}")));
                }
            }
        }
        let finished = self
            .sessions
            .iter()
            .filter_map(|(&id, session)| (session.reader_closed && session.exited).then_some(id))
            .collect::<Vec<_>>();
        for id in finished {
            self.sessions.remove(&id);
        }
        if self
            .active
            .is_some_and(|id| !self.sessions.contains_key(&id))
        {
            self.active = self.sessions.keys().next_back().copied();
        }
        exited
    }

    pub fn close_active(&mut self) -> Result<()> {
        let id = self
            .active
            .ok_or_else(|| anyhow!("no active terminal session"))?;
        if let Some(session) = self.sessions.remove(&id)
            && !session.exited
        {
            kill_and_reap(id, session.child);
        }
        self.active = self.sessions.keys().next_back().copied();
        Ok(())
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        for (id, session) in std::mem::take(&mut self.sessions) {
            if !session.exited {
                kill_and_reap(id, session.child);
            }
        }
    }
}

fn kill_and_reap(id: u64, mut child: Box<dyn Child + Send + Sync>) {
    let _ = child.kill();
    let _ = thread::Builder::new()
        .name(format!("fpsxterm-pty-reaper-{id}"))
        .spawn(move || {
            let _ = child.wait();
        });
}

fn writer_loop(mut writer: Box<dyn Write + Send>, input: std_mpsc::Receiver<Vec<u8>>) {
    while let Ok(bytes) = input.recv() {
        if writer
            .write_all(&bytes)
            .and_then(|_| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

fn reader_loop(
    mut reader: Box<dyn Read + Send>,
    session_id: u64,
    output: mpsc::Sender<RuntimeEvent>,
) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                if output
                    .blocking_send(RuntimeEvent::Output {
                        session_id,
                        bytes: buffer[..size].to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
    }
    let _ = output.blocking_send(RuntimeEvent::ReaderClosed { session_id });
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_editor_command_quotes_shell_metacharacters() {
        assert_eq!(
            shell_quote("file name;$(touch nope).txt"),
            "'file name;$(touch nope).txt'"
        );
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }
}
