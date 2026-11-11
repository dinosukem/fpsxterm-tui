use std::{
    collections::HashSet,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use glob::glob;
use serde::{Deserialize, Serialize};

const INCLUDE_PATH: &str = "~/.ssh/fpsxterm-tui/hosts.conf";
const MANAGED_HEADER: &str = "# Managed by fpsxterm-tui. Edit hosts through the application.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostSource {
    Managed,
    OpenSsh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub alias: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    pub source: HostSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedHost {
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: u16,
    pub identity_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct HostStore {
    ssh_home: PathBuf,
    config_path: PathBuf,
    managed_dir: PathBuf,
    managed_path: PathBuf,
}

impl HostStore {
    pub fn new(ssh_home: impl Into<PathBuf>, managed_dir: impl Into<PathBuf>) -> Self {
        let ssh_home = ssh_home.into();
        let managed_dir = managed_dir.into();
        Self {
            config_path: ssh_home.join("config"),
            ssh_home,
            managed_path: managed_dir.join("hosts.conf"),
            managed_dir,
        }
    }

    pub fn list(&self) -> Result<Vec<SshHost>> {
        let mut files = HashSet::new();
        let home = self.ssh_home.parent().unwrap_or(&self.ssh_home);
        let mut aliases = Vec::new();
        let mut visited = HashSet::new();
        if self.config_path.exists() {
            collect_config(&self.config_path, home, 0, &mut visited, &mut aliases)?;
        }
        if self.managed_path.exists() {
            collect_config(&self.managed_path, home, 0, &mut visited, &mut aliases)?;
        }
        let mut result = Vec::new();
        for (alias, path) in aliases {
            if !files.insert(alias.clone()) {
                continue;
            }
            let source = if path == self.managed_path {
                HostSource::Managed
            } else {
                HostSource::OpenSsh
            };
            result.push(parse_host(&alias, &path, source)?);
        }
        result.sort_by_key(|a| a.alias.to_lowercase());
        Ok(result)
    }

    pub fn save(&self, host: &ManagedHost) -> Result<()> {
        validate_host(host)?;
        let existing = self.list()?;
        if existing.iter().any(|candidate| {
            candidate.alias == host.alias && candidate.source == HostSource::OpenSsh
        }) {
            bail!(
                "SSH alias '{}' is already defined outside fpsxterm-tui",
                host.alias
            );
        }
        let mut hosts = self.read_managed()?;
        if hosts.iter().any(|candidate| candidate.alias == host.alias) {
            bail!("managed SSH alias '{}' already exists", host.alias);
        }
        hosts.push(host.clone());
        self.write_managed(&hosts)?;
        self.ensure_include()
    }

    pub fn update(&self, old_alias: &str, host: &ManagedHost) -> Result<()> {
        validate_host(host)?;
        let mut hosts = self.read_managed()?;
        let Some(index) = hosts.iter().position(|current| current.alias == old_alias) else {
            bail!("managed SSH alias '{}' does not exist", host.alias);
        };
        if old_alias != host.alias
            && self
                .list()?
                .iter()
                .any(|existing| existing.alias == host.alias)
        {
            bail!("SSH alias '{}' is already defined", host.alias);
        }
        hosts[index] = host.clone();
        self.write_managed(&hosts)
    }

    pub fn import(&self, alias: &str) -> Result<()> {
        let hosts = self.list()?;
        let Some(host) = hosts.iter().find(|host| host.alias == alias) else {
            bail!("SSH alias '{}' does not exist", alias);
        };
        if host.source != HostSource::OpenSsh {
            bail!("SSH alias '{}' is already managed by fpsxterm-tui", alias);
        }
        let source = collect_alias_sources(self)?
            .into_iter()
            .find(|(candidate, path)| candidate == alias && path != &self.managed_path)
            .map(|(_, path)| path)
            .context("could not locate source SSH config for host")?;
        validate_importable_block(&source, alias)?;
        let managed = ManagedHost {
            alias: host.alias.clone(),
            host_name: host
                .host_name
                .clone()
                .context("HostName is required to import this SSH host")?,
            user: host
                .user
                .clone()
                .context("User is required to import this SSH host")?,
            port: host.port.unwrap_or(22),
            identity_file: host.identity_file.clone(),
        };
        validate_host(&managed)?;
        let mut managed_hosts = self.read_managed()?;
        if managed_hosts.iter().any(|item| item.alias == alias) {
            bail!("SSH alias '{}' is already managed", alias);
        }
        managed_hosts.push(managed);
        self.write_managed(&managed_hosts)?;
        self.ensure_include()
    }

    pub fn delete(&self, alias: &str) -> Result<()> {
        validate_alias(alias)?;
        let mut hosts = self.read_managed()?;
        let previous_len = hosts.len();
        hosts.retain(|host| host.alias != alias);
        if hosts.len() == previous_len {
            bail!("managed SSH alias '{}' does not exist", alias);
        }
        self.write_managed(&hosts)
    }

    fn read_managed(&self) -> Result<Vec<ManagedHost>> {
        if !self.managed_path.exists() {
            return Ok(Vec::new());
        }
        let mut hosts = Vec::new();
        let content = fs::read_to_string(&self.managed_path)
            .with_context(|| format!("reading {}", self.managed_path.display()))?;
        let mut current: Option<ManagedHost> = None;
        for line in content.lines() {
            let Some((key, value)) = parse_key_value(line) else {
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "host" => {
                    if let Some(host) = current.take() {
                        hosts.push(host);
                    }
                    let aliases = shell_words::split(value).context("parsing managed Host line")?;
                    let Some(alias) = aliases.first() else {
                        bail!("empty Host entry in managed config")
                    };
                    if aliases.len() != 1 || aliases[0].contains(['*', '?', '!']) {
                        bail!("managed Host entry must contain one literal alias");
                    }
                    current = Some(ManagedHost {
                        alias: alias.clone(),
                        host_name: String::new(),
                        user: String::new(),
                        port: 22,
                        identity_file: None,
                    });
                }
                "hostname" => {
                    if let Some(host) = current.as_mut() {
                        host.host_name = parse_single(value)?;
                    }
                }
                "user" => {
                    if let Some(host) = current.as_mut() {
                        host.user = parse_single(value)?;
                    }
                }
                "port" => {
                    if let Some(host) = current.as_mut() {
                        host.port = parse_single(value)?
                            .parse()
                            .context("invalid Port in managed config")?;
                    }
                }
                "identityfile" => {
                    if let Some(host) = current.as_mut() {
                        host.identity_file = Some(PathBuf::from(parse_single(value)?));
                    }
                }
                _ => {}
            }
        }
        if let Some(host) = current {
            hosts.push(host);
        }
        for host in &hosts {
            validate_host(host)?;
        }
        Ok(hosts)
    }

    fn write_managed(&self, hosts: &[ManagedHost]) -> Result<()> {
        fs::create_dir_all(&self.managed_dir)?;
        fs::set_permissions(&self.managed_dir, fs::Permissions::from_mode(0o700))?;
        let mut content = format!("{MANAGED_HEADER}\n");
        for host in hosts {
            content.push_str(&format!(
                "\nHost {}\n    HostName {}\n    User {}\n    Port {}\n",
                quote_value(&host.alias),
                quote_value(&host.host_name),
                quote_value(&host.user),
                host.port
            ));
            if let Some(identity) = &host.identity_file {
                content.push_str(&format!(
                    "    IdentityFile {}\n",
                    quote_value(&identity.to_string_lossy())
                ));
            }
        }
        atomic_write(&self.managed_path, content.as_bytes())
    }

    fn ensure_include(&self) -> Result<()> {
        fs::create_dir_all(&self.ssh_home)?;
        let original = match fs::read(&self.config_path) {
            Ok(bytes) => {
                let metadata = fs::symlink_metadata(&self.config_path)?;
                if metadata.file_type().is_symlink() {
                    bail!("refusing to modify a symlinked SSH config")
                }
                bytes
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading {}", self.config_path.display()));
            }
        };
        let text = std::str::from_utf8(&original).context("SSH config is not valid UTF-8")?;
        let already_included = text.lines().any(|line| {
            let Some((key, value)) = parse_key_value(line) else {
                return false;
            };
            if !key.eq_ignore_ascii_case("include") {
                return false;
            }
            shell_words::split(value).is_ok_and(|paths| {
                paths.iter().any(|path| {
                    expand_home(path, self.ssh_home.parent().unwrap_or(&self.ssh_home))
                        == self.managed_path
                })
            })
        });
        if already_included {
            return Ok(());
        }
        let mut updated = format!("Include {INCLUDE_PATH}\n");
        updated.push_str(text);
        if !text.is_empty() && !text.ends_with('\n') {
            updated.push('\n');
        }
        atomic_write(&self.config_path, updated.as_bytes())?;
        Ok(())
    }
}

fn collect_alias_sources(store: &HostStore) -> Result<Vec<(String, PathBuf)>> {
    let home = store.ssh_home.parent().unwrap_or(&store.ssh_home);
    let mut aliases = Vec::new();
    let mut visited = HashSet::new();
    if store.config_path.exists() {
        collect_config(&store.config_path, home, 0, &mut visited, &mut aliases)?;
    }
    if store.managed_path.exists() {
        collect_config(&store.managed_path, home, 0, &mut visited, &mut aliases)?;
    }
    Ok(aliases)
}

fn validate_importable_block(path: &Path, alias: &str) -> Result<()> {
    let content = fs::read_to_string(path)?;
    let mut active = false;
    let mut found = false;
    for line in content.lines() {
        let Some((key, value)) = parse_key_value(line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("host") {
            active = shell_words::split(value)?
                .iter()
                .any(|candidate| candidate == alias);
            found |= active;
            continue;
        }
        if key.eq_ignore_ascii_case("match") {
            active = false;
            continue;
        }
        if active
            && !matches!(
                key.to_ascii_lowercase().as_str(),
                "hostname" | "user" | "port" | "identityfile"
            )
        {
            bail!(
                "cannot import '{}': directive '{}' needs to remain in its original OpenSSH config",
                alias,
                key
            );
        }
    }
    if !found {
        bail!("SSH Host block '{}' was not found", alias);
    }
    Ok(())
}

fn collect_config(
    path: &Path,
    home: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    aliases: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    if depth > 16 {
        bail!("SSH Include nesting exceeds 16 levels")
    }
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolving SSH config {}", path.display()))?;
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    let content = fs::read_to_string(&canonical)
        .with_context(|| format!("reading {}", canonical.display()))?;
    for line in content.lines() {
        let Some((key, value)) = parse_key_value(line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("include") {
            for raw in shell_words::split(value).context("parsing SSH Include")? {
                let pattern = if raw.starts_with("~/") || Path::new(&raw).is_absolute() {
                    expand_home(&raw, home)
                } else {
                    home.join(".ssh").join(&raw)
                };
                if pattern.to_string_lossy().contains('%') {
                    continue;
                }
                for child in glob(&pattern.to_string_lossy()).context("invalid SSH Include glob")? {
                    let child = child?;
                    if child.is_file() {
                        collect_config(&child, home, depth + 1, visited, aliases)?;
                    }
                }
            }
        } else if key.eq_ignore_ascii_case("host") {
            for alias in shell_words::split(value).context("parsing SSH Host")? {
                if alias.is_empty() || alias.contains(['*', '?', '!']) {
                    continue;
                }
                aliases.push((alias, canonical.clone()));
            }
        }
    }
    Ok(())
}

fn parse_host(alias: &str, path: &Path, source: HostSource) -> Result<SshHost> {
    let text = fs::read_to_string(path)?;
    let mut active = false;
    let mut host = SshHost {
        alias: alias.into(),
        host_name: None,
        user: None,
        port: None,
        identity_file: None,
        source,
    };
    for line in text.lines() {
        let Some((key, value)) = parse_key_value(line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("host") {
            active = shell_words::split(value)
                .is_ok_and(|aliases| aliases.iter().any(|candidate| candidate == alias));
            continue;
        }
        if key.eq_ignore_ascii_case("match") {
            active = false;
            continue;
        }
        if !active {
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "hostname" => host.host_name = first_word(value),
            "user" => host.user = first_word(value),
            "port" => host.port = first_word(value).and_then(|port| port.parse().ok()),
            "identityfile" => host.identity_file = first_word(value).map(PathBuf::from),
            _ => {}
        }
    }
    Ok(host)
}

fn validate_host(host: &ManagedHost) -> Result<()> {
    validate_alias(&host.alias)?;
    validate_value("HostName", &host.host_name)?;
    validate_value("User", &host.user)?;
    if host.host_name.trim().is_empty() || host.user.trim().is_empty() {
        bail!("HostName and User are required")
    }
    if host.port == 0 {
        bail!("Port must be between 1 and 65535")
    }
    if let Some(path) = &host.identity_file {
        validate_value("IdentityFile", &path.to_string_lossy())?;
    }
    Ok(())
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty()
        || alias.starts_with('-')
        || !alias
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        bail!(
            "Host alias must contain only ASCII letters, digits, dot, underscore or hyphen, and cannot start with '-'"
        )
    }
    Ok(())
}

fn validate_value(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        bail!("{field} is empty or contains a control character")
    }
    Ok(())
}

fn parse_key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.split('#').next()?.trim();
    if line.is_empty() {
        return None;
    }
    let (key, value) = line.split_once(char::is_whitespace)?;
    Some((key.trim(), value.trim()))
}

fn parse_single(value: &str) -> Result<String> {
    let words = shell_words::split(value)?;
    if words.len() != 1 {
        bail!("expected one value")
    }
    Ok(words[0].clone())
}

fn first_word(value: &str) -> Option<String> {
    shell_words::split(value).ok()?.into_iter().next()
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

fn quote_value(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().context("file path has no parent")?;
    fs::create_dir_all(parent)?;
    let permissions = fs::symlink_metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("refusing to replace symlink {}", path.display())
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(content)?;
    temp.as_file().sync_all()?;
    if let Some(permissions) = permissions {
        temp.as_file().set_permissions(permissions)?;
    }
    temp.persist(path)
        .with_context(|| format!("atomically replacing {}", path.display()))?;
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_literal_hosts_across_include_and_ignores_patterns() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        let managed = ssh.join("fpsxterm-tui");
        fs::create_dir_all(&managed).unwrap();
        fs::write(ssh.join("config"), "Host *.example.com\n  User wildcard\nInclude conf.d/*\nHost prod # comment\n  HostName prod.example.com\n").unwrap();
        fs::create_dir_all(ssh.join("conf.d")).unwrap();
        fs::write(
            ssh.join("conf.d/extra"),
            "Host stage dev-box\n  User deploy\n",
        )
        .unwrap();
        let store = HostStore::new(&ssh, &managed);
        let hosts = store.list().unwrap();
        assert_eq!(
            hosts
                .iter()
                .map(|host| host.alias.as_str())
                .collect::<Vec<_>>(),
            ["dev-box", "prod", "stage"]
        );
        assert_eq!(hosts[1].host_name.as_deref(), Some("prod.example.com"));
    }

    #[test]
    fn save_preserves_existing_config_and_inserts_include_once() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        let managed = ssh.join("fpsxterm-tui");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(
            ssh.join("config"),
            "Host existing\n  HostName example.com\n",
        )
        .unwrap();
        let store = HostStore::new(&ssh, &managed);
        let host = ManagedHost {
            alias: "work".into(),
            host_name: "server.example.com".into(),
            user: "deploy".into(),
            port: 2222,
            identity_file: Some(PathBuf::from("~/.ssh/id_ed25519")),
        };
        store.save(&host).unwrap();
        store
            .save(&ManagedHost {
                alias: "stage".into(),
                host_name: "stage.example.com".into(),
                user: "deploy".into(),
                port: 22,
                identity_file: None,
            })
            .unwrap();
        let config = fs::read_to_string(ssh.join("config")).unwrap();
        assert_eq!(
            config
                .matches("Include ~/.ssh/fpsxterm-tui/hosts.conf")
                .count(),
            1
        );
        assert!(config.contains("Host existing"));
        assert_eq!(store.list().unwrap().len(), 3);
        assert_eq!(
            fs::metadata(managed.join("hosts.conf"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn rejects_invalid_alias_and_external_host_collision() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        let managed = ssh.join("fpsxterm-tui");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(
            ssh.join("config"),
            "Host production\n  HostName prod.example.com\n",
        )
        .unwrap();
        let store = HostStore::new(&ssh, &managed);
        let invalid = ManagedHost {
            alias: "-oProxyCommand=bad".into(),
            host_name: "host".into(),
            user: "user".into(),
            port: 22,
            identity_file: None,
        };
        assert!(store.save(&invalid).is_err());
        let collision = ManagedHost {
            alias: "production".into(),
            host_name: "other.example.com".into(),
            user: "user".into(),
            port: 22,
            identity_file: None,
        };
        assert!(store.save(&collision).is_err());
        assert_eq!(
            fs::read_to_string(ssh.join("config")).unwrap(),
            "Host production\n  HostName prod.example.com\n"
        );
    }

    #[test]
    fn import_copies_supported_host_without_rewriting_original_config() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        let managed = ssh.join("fpsxterm-tui");
        fs::create_dir_all(&ssh).unwrap();
        let config_path = ssh.join("config");
        let original = "Host build\n  HostName build.example.test\n  User deploy\n  Port 2202\n";
        fs::write(&config_path, original).unwrap();
        let store = HostStore::new(&ssh, &managed);
        store.import("build").unwrap();
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            format!("Include {INCLUDE_PATH}\n{original}")
        );
        let imported = store
            .list()
            .unwrap()
            .into_iter()
            .find(|host| host.alias == "build")
            .unwrap();
        assert_eq!(imported.source, HostSource::Managed);
        assert_eq!(imported.port, Some(2202));
    }

    #[test]
    fn refuses_to_import_hosts_with_unmanaged_directives() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(
            ssh.join("config"),
            "Host jump\n  HostName jump.example.test\n  User deploy\n  ProxyJump gateway\n",
        )
        .unwrap();
        let store = HostStore::new(&ssh, ssh.join("fpsxterm-tui"));
        assert!(
            store
                .import("jump")
                .unwrap_err()
                .to_string()
                .contains("ProxyJump")
        );
    }
}
