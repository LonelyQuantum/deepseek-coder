#![forbid(unsafe_code)]

use deepseek_coder_agent_core::PROJECT_NAME;

pub const JSON_RPC_VERSION: &str = "2.0";
pub const PROTOCOL_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcMethod {
    pub name: &'static str,
}

impl RpcMethod {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub fn qualified_name(self) -> String {
        format!("{PROJECT_NAME}/{}", self.name)
    }
}

pub const INITIALIZE_METHOD: RpcMethod = RpcMethod::new("initialize");

#[cfg(test)]
mod tests {
    use super::INITIALIZE_METHOD;

    #[test]
    fn method_names_are_project_scoped() {
        assert_eq!(
            INITIALIZE_METHOD.qualified_name(),
            "deepseek-coder/initialize"
        );
    }
}
