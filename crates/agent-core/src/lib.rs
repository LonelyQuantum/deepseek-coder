#![forbid(unsafe_code)]

pub mod approval;
pub mod cancellation;
pub mod context;
mod hashing;
pub mod provider;
pub mod reasoning;
pub mod run_log;
#[doc(hidden)]
pub mod test_helpers;
pub mod tool;
pub mod tool_execution;
pub mod turn_loop;
pub mod workspace_manifest;

pub const PROJECT_NAME: &str = "deepseek-coder";
pub const DEFAULT_STATE_DIR: &str = ".deepseek-coder";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentMetadata {
    pub name: &'static str,
    pub state_dir: &'static str,
}

impl AgentMetadata {
    pub const fn new(name: &'static str, state_dir: &'static str) -> Self {
        Self { name, state_dir }
    }
}

pub const AGENT_METADATA: AgentMetadata = AgentMetadata::new(PROJECT_NAME, DEFAULT_STATE_DIR);

#[cfg(test)]
mod tests {
    use super::{AGENT_METADATA, DEFAULT_STATE_DIR, PROJECT_NAME};

    #[test]
    fn metadata_uses_project_defaults() {
        assert_eq!(AGENT_METADATA.name, PROJECT_NAME);
        assert_eq!(AGENT_METADATA.state_dir, DEFAULT_STATE_DIR);
    }
}
