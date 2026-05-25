use crate::approval::{ApprovalRequirement, RiskLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    WorkspaceManifest,
    ReadFile,
    Search,
    ApplyPatch,
    Shell,
    GitStatus,
    GitDiff,
    LspDiagnostics,
    PlanUpdate,
}

impl ToolName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceManifest => "workspace_manifest",
            Self::ReadFile => "read_file",
            Self::Search => "search",
            Self::ApplyPatch => "apply_patch",
            Self::Shell => "shell",
            Self::GitStatus => "git_status",
            Self::GitDiff => "git_diff",
            Self::LspDiagnostics => "lsp_diagnostics",
            Self::PlanUpdate => "plan_update",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolImplementationStatus {
    SchemaOnly,
    ExecutorImplemented,
}

impl ToolImplementationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaOnly => "schema_only",
            Self::ExecutorImplemented => "executor_implemented",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub description: &'static str,
    pub risk: RiskLevel,
    pub approval: ApprovalRequirement,
    pub implementation_status: ToolImplementationStatus,
    pub argument_schema: &'static str,
    pub result_schema: &'static str,
}

impl ToolDefinition {
    pub const fn new(
        name: ToolName,
        description: &'static str,
        risk: RiskLevel,
        approval: ApprovalRequirement,
        implementation_status: ToolImplementationStatus,
        argument_schema: &'static str,
        result_schema: &'static str,
    ) -> Self {
        Self {
            name,
            description,
            risk,
            approval,
            implementation_status,
            argument_schema,
            result_schema,
        }
    }
}

const STATUS_RESULT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["status", "summary"],
  "properties": {
    "status": { "type": "string", "enum": ["ok", "failed"] },
    "summary": { "type": "string" },
    "errorCode": { "type": "string" }
  }
}"#;

const WORKSPACE_MANIFEST_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "root": { "type": "string", "minLength": 1 },
    "respectGitignore": { "type": "boolean" },
    "maxEntries": { "type": "integer", "minimum": 1 }
  }
}"#;

const WORKSPACE_MANIFEST_RESULT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["status", "summary", "manifestHash", "summaryMarkdown", "manifest"],
  "properties": {
    "status": { "type": "string", "enum": ["ok", "failed"] },
    "summary": { "type": "string" },
    "errorCode": { "type": "string" },
    "manifestHash": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
    "summaryMarkdown": { "type": "string" },
    "manifest": {
      "type": "object",
      "additionalProperties": true,
      "required": ["manifestVersion", "manifestHash", "maxEntries", "totalDiscoveredFiles", "includedFiles", "entries", "omitted"],
      "properties": {
        "manifestVersion": { "type": "integer", "minimum": 1 },
        "manifestHash": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
        "workspaceRoot": { "type": "string" },
        "scanRoot": { "type": "string" },
        "maxEntries": { "type": "integer", "minimum": 1 },
        "totalDiscoveredFiles": { "type": "integer", "minimum": 0 },
        "includedFiles": { "type": "integer", "minimum": 0 },
        "totalSizeBytes": { "type": "integer", "minimum": 0 },
        "entries": { "type": "array", "items": { "type": "object" } },
        "omitted": { "type": "array", "items": { "type": "object" } }
      }
    }
  }
}"#;

const READ_FILE_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["path"],
  "properties": {
    "path": { "type": "string", "minLength": 1 },
    "startLine": { "type": "integer", "minimum": 1 },
    "endLine": { "type": "integer", "minimum": 1 }
  }
}"#;

const READ_FILE_RESULT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["status", "summary", "path", "content", "lineCount", "sha256", "sizeBytes"],
  "properties": {
    "status": { "type": "string", "enum": ["ok", "failed"] },
    "summary": { "type": "string" },
    "errorCode": { "type": "string" },
    "path": { "type": "string" },
    "content": { "type": "string" },
    "lineCount": { "type": "integer", "minimum": 0 },
    "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
    "sizeBytes": { "type": "integer", "minimum": 0 }
  }
}"#;

const SEARCH_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["query"],
  "properties": {
    "query": { "type": "string", "minLength": 1 },
    "paths": { "type": "array", "items": { "type": "string" } },
    "caseSensitive": { "type": "boolean" },
    "maxResults": { "type": "integer", "minimum": 1 }
  }
}"#;

const APPLY_PATCH_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["unifiedDiff", "expectedFiles"],
  "properties": {
    "unifiedDiff": { "type": "string", "minLength": 1 },
    "expectedFiles": {
      "type": "array",
      "minItems": 1,
      "items": { "type": "string", "minLength": 1 }
    }
  }
}"#;

const SHELL_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["command"],
  "properties": {
    "command": { "type": "string", "minLength": 1 },
    "cwd": { "type": "string" },
    "timeoutMs": { "type": "integer", "minimum": 1 }
  }
}"#;

const GIT_STATUS_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "porcelain": { "type": "boolean" }
  }
}"#;

const GIT_DIFF_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "staged": { "type": "boolean" },
    "paths": { "type": "array", "items": { "type": "string" } }
  }
}"#;

const LSP_DIAGNOSTICS_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "paths": { "type": "array", "items": { "type": "string" } }
  }
}"#;

const PLAN_UPDATE_ARGUMENT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["steps"],
  "properties": {
    "steps": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "status"],
        "properties": {
          "id": { "type": "string" },
          "title": { "type": "string" },
          "status": {
            "type": "string",
            "enum": ["pending", "in_progress", "completed", "failed", "canceled"]
          },
          "detail": { "type": "string" }
        }
      }
    }
  }
}"#;

pub const BUILTIN_TOOLS: &[ToolDefinition] = &[
    ToolDefinition::new(
        ToolName::WorkspaceManifest,
        "Generate the workspace manifest.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::ExecutorImplemented,
        WORKSPACE_MANIFEST_ARGUMENT_SCHEMA,
        WORKSPACE_MANIFEST_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::ReadFile,
        "Read a UTF-8 text file from the workspace.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::ExecutorImplemented,
        READ_FILE_ARGUMENT_SCHEMA,
        READ_FILE_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::Search,
        "Search workspace text with ripgrep.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::ExecutorImplemented,
        SEARCH_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::ApplyPatch,
        "Apply a unified diff patch.",
        RiskLevel::Write,
        ApprovalRequirement::Required,
        ToolImplementationStatus::ExecutorImplemented,
        APPLY_PATCH_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::Shell,
        "Execute a non-interactive shell command.",
        RiskLevel::Exec,
        ApprovalRequirement::Required,
        ToolImplementationStatus::ExecutorImplemented,
        SHELL_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::GitStatus,
        "Read git status.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::ExecutorImplemented,
        GIT_STATUS_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::GitDiff,
        "Read git diff.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::ExecutorImplemented,
        GIT_DIFF_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::LspDiagnostics,
        "Read language-server diagnostics.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::SchemaOnly,
        LSP_DIAGNOSTICS_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
    ToolDefinition::new(
        ToolName::PlanUpdate,
        "Update the active plan.",
        RiskLevel::Read,
        ApprovalRequirement::None,
        ToolImplementationStatus::SchemaOnly,
        PLAN_UPDATE_ARGUMENT_SCHEMA,
        STATUS_RESULT_SCHEMA,
    ),
];

pub fn find_builtin_tool(name: &str) -> Option<&'static ToolDefinition> {
    BUILTIN_TOOLS.iter().find(|tool| tool.name.as_str() == name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    use super::{BUILTIN_TOOLS, ToolName, find_builtin_tool};
    use crate::approval::{ALL_RISK_LEVELS, ApprovalRequirement, RiskLevel};

    #[test]
    fn all_builtin_tools_have_matching_default_approval() {
        for tool in BUILTIN_TOOLS {
            assert_eq!(
                tool.approval,
                tool.risk.default_approval(),
                "tool {} has mismatched approval requirement",
                tool.name.as_str()
            );
        }
    }

    #[test]
    fn write_and_exec_tools_require_approval() {
        let apply_patch = find_builtin_tool(ToolName::ApplyPatch.as_str())
            .expect("apply_patch tool must be registered");
        let shell =
            find_builtin_tool(ToolName::Shell.as_str()).expect("shell tool must be registered");

        assert_eq!(apply_patch.risk, RiskLevel::Write);
        assert_eq!(apply_patch.approval, ApprovalRequirement::Required);
        assert_eq!(shell.risk, RiskLevel::Exec);
        assert_eq!(shell.approval, ApprovalRequirement::Required);
    }

    #[test]
    fn schemas_are_explicit_objects() {
        for tool in BUILTIN_TOOLS {
            assert!(
                tool.argument_schema.contains("\"type\": \"object\""),
                "tool {} argument schema must be an object",
                tool.name.as_str()
            );
            assert!(
                tool.result_schema.contains("\"status\""),
                "tool {} result schema must include status",
                tool.name.as_str()
            );
        }
    }

    #[test]
    fn read_file_result_schema_exposes_file_summary_metadata() {
        let read_file = find_builtin_tool(ToolName::ReadFile.as_str())
            .expect("read_file tool must be registered");

        assert!(read_file.result_schema.contains("\"sha256\""));
        assert!(read_file.result_schema.contains("\"sizeBytes\""));
        assert!(
            read_file
                .result_schema
                .contains("\"pattern\": \"^[0-9a-f]{64}$\"")
        );
    }

    #[test]
    fn workspace_manifest_result_schema_exposes_manifest_metadata() {
        let workspace_manifest = find_builtin_tool(ToolName::WorkspaceManifest.as_str())
            .expect("workspace_manifest tool must be registered");

        assert_eq!(
            workspace_manifest.implementation_status,
            super::ToolImplementationStatus::ExecutorImplemented
        );
        assert!(
            workspace_manifest
                .result_schema
                .contains("\"manifestHash\"")
        );
        assert!(
            workspace_manifest
                .result_schema
                .contains("\"summaryMarkdown\"")
        );
        assert!(
            workspace_manifest
                .result_schema
                .contains("\"pattern\": \"^sha256:[0-9a-f]{64}$\"")
        );
    }

    #[test]
    fn builtin_tools_match_protocol_fixture() {
        let fixture: ToolRegistryFixture = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/protocol/tool-registry.v1.json"
        )))
        .expect("tool registry fixture should parse");

        assert_eq!(fixture.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            fixture.risk_levels,
            ALL_RISK_LEVELS
                .iter()
                .map(|risk| risk.as_str().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            fixture.risk_default_approval,
            ALL_RISK_LEVELS
                .iter()
                .map(|risk| {
                    (
                        risk.as_str().to_owned(),
                        risk.default_approval().as_str().to_owned(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        );
        let expected_tools = BUILTIN_TOOLS
            .iter()
            .map(|tool| ToolFixture {
                name: tool.name.as_str().to_owned(),
                risk: tool.risk.as_str().to_owned(),
                approval: tool.approval.as_str().to_owned(),
                status: tool.implementation_status.as_str().to_owned(),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            tool_fixture_map(fixture.tools),
            tool_fixture_map(expected_tools)
        );
    }

    fn tool_fixture_map(tools: Vec<ToolFixture>) -> BTreeMap<String, ToolFixture> {
        let tool_count = tools.len();
        let tool_map = tools
            .into_iter()
            .map(|tool| (tool.name.clone(), tool))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            tool_count,
            tool_map.len(),
            "tool fixture names must be unique"
        );
        tool_map
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ToolRegistryFixture {
        version: String,
        risk_levels: Vec<String>,
        risk_default_approval: BTreeMap<String, String>,
        tools: Vec<ToolFixture>,
    }

    #[derive(Debug, PartialEq, Eq, Deserialize)]
    struct ToolFixture {
        name: String,
        risk: String,
        approval: String,
        status: String,
    }
}
