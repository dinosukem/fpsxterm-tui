use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub terminal: TerminalConfig,
    pub files: FilesConfig,
    pub transfer: TransferConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub mouse_enabled: bool,
    pub show_hidden_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub shell: PathBuf,
    pub editor: Vec<String>,
    pub scrollback_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesConfig {
    pub confirm_delete: bool,
    pub confirm_overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferConfig {
    pub max_concurrent: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            mouse_enabled: true,
            show_hidden_files: false,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        let shell = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/bash"));
        Self {
            shell,
            editor: vec!["nano".into()],
            scrollback_lines: 10_000,
        }
    }
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            confirm_delete: true,
            confirm_overwrite: true,
        }
    }
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self { max_concurrent: 2 }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    config_path: PathBuf,
    ssh_dir: PathBuf,
    state_dir: PathBuf,
}

impl ConfigStore {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("com", "fpsxterm", "fpsxterm-tui")
            .context("could not determine XDG application directories")?;
        let home = BaseDirs::new()
            .context("could not determine the home directory")?
            .home_dir()
            .to_path_buf();
        Ok(Self {
            config_path: dirs.config_dir().join("config.toml"),
            ssh_dir: home.join(".ssh").join("fpsxterm-tui"),
            state_dir: dirs.state_dir().unwrap_or(dirs.config_dir()).to_path_buf(),
        })
    }

    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }
    pub fn ssh_dir(&self) -> &std::path::Path {
        &self.ssh_dir
    }
    pub fn history_path(&self) -> PathBuf {
        self.state_dir.join("history.toml")
    }
    pub fn managed_hosts_path(&self) -> PathBuf {
        self.ssh_dir.join("hosts.conf")
    }
    pub fn ssh_config_path(&self) -> PathBuf {
        self.ssh_dir.parent().expect(".ssh parent").join("config")
    }

    pub fn load(&self) -> Result<AppConfig> {
        if fs::symlink_metadata(&self.config_path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            anyhow::bail!("refusing to read config through a symbolic link");
        }
        match fs::read_to_string(&self.config_path) {
            Ok(contents) => toml::from_str(&contents).context("parsing config.toml"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = AppConfig::default();
                self.save(&config).with_context(|| {
                    format!("creating default config at {}", self.config_path.display())
                })?;
                Ok(config)
            }
            Err(error) => {
                Err(error).with_context(|| format!("reading {}", self.config_path.display()))
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        let parent = self
            .config_path
            .parent()
            .context("config directory has no parent")?;
        fs::create_dir_all(parent)?;
        if fs::symlink_metadata(&self.config_path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            anyhow::bail!("refusing to replace config through a symbolic link");
        }
        let contents = toml::to_string_pretty(config)?;
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        fs::write(temp.path(), contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o600))?;
        }
        temp.persist(&self.config_path)
            .context("atomically replacing config.toml")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore {
            config_path: dir.path().join("config.toml"),
            ssh_dir: dir.path().join(".ssh/fpsxterm-tui"),
            state_dir: dir.path().join("state"),
        };
        let config = store.load().unwrap();
        assert!(store.config_path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.config_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(config.terminal.editor, ["nano"]);
        assert_eq!(config.transfer.max_concurrent, 2);
        store.save(&config).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.files.confirm_delete);
        assert_eq!(loaded.terminal.scrollback_lines, 10_000);
        assert!(!loaded.ui.show_hidden_files);
    }

    #[test]
    fn show_hidden_files_can_be_enabled_or_disabled_in_config() {
        let enabled: AppConfig = toml::from_str("[ui]\nshow_hidden_files = true").unwrap();
        let disabled: AppConfig = toml::from_str("[ui]\nshow_hidden_files = false").unwrap();

        assert!(enabled.ui.show_hidden_files);
        assert!(!disabled.ui.show_hidden_files);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_config_files() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        fs::write(&target, "[ui]\nshow_hidden_files = true\n").unwrap();
        let config_path = dir.path().join("config.toml");
        symlink(&target, &config_path).unwrap();
        let store = ConfigStore {
            config_path,
            ssh_dir: dir.path().join(".ssh/fpsxterm-tui"),
            state_dir: dir.path().join("state"),
        };

        assert!(store.load().is_err());
        assert!(store.save(&AppConfig::default()).is_err());
        assert!(
            fs::read_to_string(target)
                .unwrap()
                .contains("show_hidden_files = true")
        );
    }
}
