use {
    serde::{
        Deserialize,
        Serialize,
    },
    std::path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostSource {
    Managed,
    OpenSsh,
    Docker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub alias: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    pub source: HostSource,
    pub docker: Option<crate::config::DockerHostConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedHost {
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: u16,
    pub identity_file: Option<PathBuf>,
}
