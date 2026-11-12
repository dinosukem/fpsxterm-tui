#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    HostList,
    Terminal,
    LocalFiles,
    RemoteFiles,
    Dialog,
}
