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
    std::{
        path::{
            Path,
            PathBuf,
        },
        process::Stdio,
        time::{
            Duration,
            SystemTime,
            UNIX_EPOCH,
        },
    },
    tokio::process::Command,
};

const LIST_SCRIPT: &str = r#"
for path in "$1"/* "$1"/.[!.]* "$1"/..?*; do
    [ -e "$path" ] || [ -L "$path" ] || continue
    name=${path##*/}
    if [ -L "$path" ]; then kind=l
    elif [ -d "$path" ]; then kind=d
    elif [ -f "$path" ]; then kind=f
    else kind=o
    fi
    size=$(stat -c %s "$path" 2>/dev/null) || size=
    modified=$(stat -c %Y "$path" 2>/dev/null) || modified=
    printf '%s\0%s\0%s\0%s\0' "$name" "$kind" "$size" "$modified"
done
"#;
const REQUIRED_DOCKER_TOOLS: &[&str] = &[
    "sh", "cat", "mkdir", "mv", "rm", "rmdir", "ln", "mktemp", "wc",
];

#[derive(Clone)]
pub struct DockerFs {
    control_path: PathBuf,
    ssh_host: String,
    container: String,
}

impl DockerFs {
    pub fn new(control_path: PathBuf, ssh_host: String, container: String) -> Self {
        Self {
            control_path,
            ssh_host,
            container,
        }
    }

    fn remote_command(&self, script: &str, args: &[String], stdin: bool) -> String {
        let mut command = vec!["docker".to_owned(), "exec".to_owned()];
        if stdin {
            command.push("-i".into());
        }
        command.push(self.container.clone());
        command.extend([
            "sh".into(),
            "-c".into(),
            shell_words::quote(script).into_owned(),
            "sh".into(),
        ]);
        command.extend(args.iter().map(|arg| shell_words::quote(arg).into_owned()));
        command.join(" ")
    }

    fn ssh(&self, script: &str, args: &[String], stdin: bool) -> Command {
        let mut command = Command::new("ssh");
        command
            .args([
                "-T",
                "-S",
                self.control_path.to_string_lossy().as_ref(),
                "-o",
                "BatchMode=yes",
                "-o",
                "NumberOfPasswordPrompts=0",
                "--",
                &self.ssh_host,
            ])
            .arg(self.remote_command(script, args, stdin));
        command
    }

    async fn output(&self, script: &str, args: &[String]) -> Result<Vec<u8>> {
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            self.ssh(script, args, false).output(),
        )
        .await
        .context("container file operation timed out")??;
        if !output.status.success() {
            bail!("{}", diagnostic(&output.stderr, output.status.to_string()));
        }
        Ok(output.stdout)
    }

    pub async fn home(&self) -> Result<PathBuf> {
        let output = self.output("printf '%s' \"${HOME:-/}\"", &[]).await?;
        let path = String::from_utf8(output).context("container home is not UTF-8")?;
        Ok(PathBuf::from(if path.is_empty() { "/" } else { &path }))
    }

    pub async fn list(&self, path: &Path) -> Result<Vec<FileEntry>> {
        let output = self
            .output(LIST_SCRIPT, &[path.display().to_string()])
            .await?;
        let mut entries = parse_list_output(path, &output);
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Directory)
                .cmp(&(b.kind != EntryKind::Directory))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub async fn read_file(&self, path: &Path) -> Result<String> {
        let mut command = self.ssh("cat -- \"$1\"", &[path.display().to_string()], false);
        let output = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .context("container viewer timed out")??;
        if !output.status.success() {
            bail!("{}", diagnostic(&output.stderr, output.status.to_string()));
        }
        if output.stdout.len() > 2 * 1024 * 1024 {
            bail!("viewer limit is 2 MiB");
        }
        String::from_utf8(output.stdout).context("file is not valid UTF-8 text")
    }

    pub async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = safe_child(parent, name)?;
        self.output("mkdir -- \"$1\"", &[path.display().to_string()])
            .await?;
        Ok(path)
    }

    pub async fn ensure_dir(&self, path: &Path) -> Result<()> {
        self.output("mkdir -p -- \"$1\"", &[path.display().to_string()])
            .await?;
        Ok(())
    }

    pub async fn rename(&self, path: &Path, name: &str) -> Result<PathBuf> {
        let parent = path.parent().context("entry has no parent directory")?;
        let destination = safe_child(parent, name)?;
        self.output(
            "test ! -e \"$2\" && test ! -L \"$2\" && mv -- \"$1\" \"$2\"",
            &[
                path.display().to_string(),
                destination.display().to_string(),
            ],
        )
        .await?;
        Ok(destination)
    }

    pub async fn delete(&self, path: &Path, kind: EntryKind) -> Result<()> {
        if kind != EntryKind::Directory {
            self.output("rm -f -- \"$1\"", &[path.display().to_string()])
                .await?;
            return Ok(());
        }
        let mut pending = vec![path.to_path_buf()];
        let mut directories = Vec::new();
        while let Some(directory) = pending.pop() {
            directories.push(directory.clone());
            for entry in self.list(&directory).await? {
                if entry.kind == EntryKind::Directory {
                    pending.push(entry.path);
                } else {
                    self.output("rm -f -- \"$1\"", &[entry.path.display().to_string()])
                        .await?;
                }
            }
        }
        for directory in directories.into_iter().rev() {
            self.output("rmdir -- \"$1\"", &[directory.display().to_string()])
                .await?;
        }
        Ok(())
    }

    pub async fn diagnostics(&self) -> Result<Vec<String>> {
        let mut missing = Vec::new();
        for tool in REQUIRED_DOCKER_TOOLS {
            if self
                .output("command -v \"$1\" >/dev/null 2>&1", &[(*tool).into()])
                .await
                .is_err()
            {
                missing.push((*tool).to_owned());
            }
        }
        Ok(missing)
    }

    pub async fn tool_install_hint(&self, missing: &[String]) -> Result<Option<String>> {
        let release = self
            .output("cat /etc/os-release 2>/dev/null || true", &[])
            .await
            .unwrap_or_default();
        Ok(Self::install_suggestion(
            missing,
            &String::from_utf8_lossy(&release),
        ))
    }

    pub fn install_suggestion(missing: &[String], os_release: &str) -> Option<String> {
        if missing.is_empty() {
            return None;
        }
        let lower = os_release.to_ascii_lowercase();
        let packages = missing
            .iter()
            .map(|tool| {
                match tool.as_str() {
                    "sh" if lower.contains("alpine") => "busybox",
                    "sh" if lower.contains("debian") || lower.contains("ubuntu") => "dash",
                    "sh" if lower.contains("fedora")
                        || lower.contains("rhel")
                        || lower.contains("centos") =>
                    {
                        "bash"
                    }
                    "cat" | "mkdir" | "mv" | "rm" | "rmdir" | "ln" | "wc" | "mktemp" => "coreutils",
                    "nano" => "nano",
                    other => other,
                }
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" ");
        let command = if lower.contains("alpine") {
            format!("apk add {packages}")
        } else if lower.contains("debian") || lower.contains("ubuntu") {
            format!("apt-get update && apt-get install -y {packages}")
        } else if lower.contains("fedora") || lower.contains("rhel") || lower.contains("centos") {
            format!("dnf install -y {packages}")
        } else {
            format!("install these tools with the container's package manager: {packages}")
        };
        Some(command)
    }

    pub async fn open_read(&self, path: &Path) -> Result<tokio::process::Child> {
        let mut command = self.ssh("cat -- \"$1\"", &[path.display().to_string()], false);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command.spawn().context("starting container file download")
    }

    pub async fn open_write(
        &self,
        path: &Path,
        overwrite: bool,
        expected_size: u64,
    ) -> Result<tokio::process::Child> {
        let script = "umask 077; tmp=$(mktemp \"${1%/*}/.fpsxterm-XXXXXXXX\") || exit; trap 'rm -f -- \"$tmp\"' EXIT HUP INT TERM; cat > \"$tmp\" || exit; actual=$(wc -c < \"$tmp\") || exit; test \"$actual\" = \"$3\" || exit 1; if [ \"$2\" = 1 ]; then mv -f -- \"$tmp\" \"$1\" || exit; else ln -- \"$tmp\" \"$1\" || exit; rm -f -- \"$tmp\" || exit; fi; trap - EXIT HUP INT TERM";
        let mut command = self.ssh(
            script,
            &[
                path.display().to_string(),
                if overwrite { "1" } else { "0" }.into(),
                expected_size.to_string(),
            ],
            true,
        );
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command.spawn().context("starting container file upload")
    }
}

fn parse_epoch(value: &[u8]) -> Option<SystemTime> {
    let seconds = std::str::from_utf8(value).ok()?.parse::<f64>().ok()?;
    (seconds.is_finite() && seconds >= 0.0)
        .then(|| UNIX_EPOCH.checked_add(Duration::from_secs_f64(seconds)))
        .flatten()
}

fn parse_list_output(path: &Path, output: &[u8]) -> Vec<FileEntry> {
    let mut fields = output.split(|byte| *byte == 0);
    let mut entries = Vec::new();
    while let Some(name) = fields.next() {
        if name.is_empty() {
            continue;
        }
        let kind = fields.next().unwrap_or_default();
        let size = fields.next().unwrap_or_default();
        let modified = fields.next().unwrap_or_default();
        let name = String::from_utf8_lossy(name).into_owned();
        entries.push(FileEntry {
            path: path.join(&name),
            kind: match kind {
                b"d" => EntryKind::Directory,
                b"f" => EntryKind::File,
                b"l" => EntryKind::Symlink,
                _ => EntryKind::Other,
            },
            size: std::str::from_utf8(size)
                .ok()
                .and_then(|size| size.parse().ok()),
            modified: parse_epoch(modified),
            name,
        });
    }
    entries
}

fn diagnostic(stderr: &[u8], fallback: String) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        fallback
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_script_returns_visible_hidden_and_directory_entries_without_gnu_find() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("file with spaces.txt"), "data")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join(".hidden"), "secret")
            .await
            .unwrap();
        tokio::fs::create_dir(dir.path().join("folder"))
            .await
            .unwrap();

        let output = Command::new("sh")
            .args(["-c", LIST_SCRIPT, "sh"])
            .arg(dir.path())
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        let entries = parse_list_output(dir.path(), &output.stdout);

        assert!(entries.iter().any(|entry| {
            entry.name == "file with spaces.txt"
                && entry.kind == EntryKind::File
                && entry.size == Some(4)
        }));
        assert!(
            entries
                .iter()
                .any(|entry| { entry.name == ".hidden" && entry.kind == EntryKind::File })
        );
        assert!(
            entries
                .iter()
                .any(|entry| { entry.name == "folder" && entry.kind == EntryKind::Directory })
        );
    }

    #[test]
    fn docker_file_tools_do_not_require_a_terminal_editor() {
        assert!(!REQUIRED_DOCKER_TOOLS.contains(&"nano"));
    }
}
