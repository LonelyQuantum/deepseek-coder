use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

pub use crate::token_estimator::TokenEstimatorReport;

use crate::{
    hashing::sha256_hex,
    run_log::redact_text,
    token_estimator::{TokenEstimator, TokenEstimatorConfig, TokenEstimatorError},
};

const DEFAULT_MAX_INPUT_TOKENS: u64 = 1_000_000;
const DEFAULT_STABLE_PREFIX_BUDGET_RATIO_PPM: u32 = 300_000;
const PPM_SCALE: u32 = 1_000_000;

#[derive(Debug, Clone)]
pub struct ContextBuilder {
    config: ContextBuilderConfig,
    items: Vec<ContextItem>,
    manifest_report: Option<ContextManifestReport>,
}

impl ContextBuilder {
    pub fn new(config: ContextBuilderConfig) -> Self {
        Self {
            config,
            items: Vec::new(),
            manifest_report: None,
        }
    }

    pub fn add_item(&mut self, item: ContextItem) -> &mut Self {
        self.items.push(item);
        self
    }

    pub fn with_item(mut self, item: ContextItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn set_manifest_report(&mut self, manifest_report: ContextManifestReport) -> &mut Self {
        self.manifest_report = Some(manifest_report);
        self
    }

    pub fn with_manifest_report(mut self, manifest_report: ContextManifestReport) -> Self {
        self.manifest_report = Some(manifest_report);
        self
    }

    pub fn build(&self) -> Result<ContextCapsule, ContextBuildError> {
        if self.config.max_input_tokens == 0 {
            return Err(ContextBuildError::InvalidMaxInputTokens);
        }
        if self.config.stable_prefix_budget_ratio_ppm == 0
            || self.config.stable_prefix_budget_ratio_ppm > PPM_SCALE
        {
            return Err(ContextBuildError::InvalidStablePrefixBudgetRatio {
                ratio_ppm: self.config.stable_prefix_budget_ratio_ppm,
            });
        }

        let mut indexed_items = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| IndexedContextItem { index, item })
            .collect::<Vec<_>>();
        indexed_items.sort_by_key(|item| {
            let placement = item
                .item
                .cache_placement
                .unwrap_or_else(|| CachePlacement::default_for_kind(item.item.kind));
            (placement.order(), item.item.kind.priority(), item.index)
        });

        let mut included_items = Vec::new();
        let mut included_sources = Vec::new();
        let mut omitted_sources = Vec::new();
        let mut seen_singletons = HashSet::new();
        let mut seen_file_paths = HashSet::new();
        let mut seen_command_ids = HashSet::new();

        for indexed_item in indexed_items {
            let prepared = prepare_item(indexed_item.item)?;
            validate_unique_source(
                &prepared.source,
                &mut seen_singletons,
                &mut seen_file_paths,
                &mut seen_command_ids,
            )?;
            let section_item = section_item_from_prepared(&prepared, &self.config.token_estimator)?;
            let item_tokens = section_item.tokens;
            let mut candidate_items = included_items.clone();
            candidate_items.push(section_item.clone());
            let candidate_sections =
                build_sections(candidate_items.clone(), &self.config.token_estimator)?;
            let candidate_rendered = render_context_sections(&candidate_sections);
            let candidate_input_tokens =
                estimate_text(&self.config.token_estimator, &candidate_rendered)?;

            if candidate_input_tokens > self.config.max_input_tokens {
                if prepared.source.required {
                    return Err(ContextBuildError::RequiredContextExceedsBudget {
                        max_input_tokens: self.config.max_input_tokens,
                        used_tokens: estimate_text(
                            &self.config.token_estimator,
                            &render_context_sections(&build_sections(
                                included_items.clone(),
                                &self.config.token_estimator,
                            )?),
                        )?,
                        required_tokens: candidate_input_tokens,
                        context_source: prepared.source,
                    });
                }

                omitted_sources.push(ContextOmittedSource {
                    source: prepared.source,
                    estimated_tokens: item_tokens,
                    inclusion_reason: prepared.inclusion_reason,
                    omission_reason: ContextOmissionReason::TokenBudgetExceeded,
                });
                continue;
            }

            let stable_prefix_budget_tokens = self.config.stable_prefix_budget_tokens();
            let candidate_stable_prefix_tokens =
                section_tokens(&candidate_sections, CachePlacement::StablePrefix);
            if prepared.placement == CachePlacement::StablePrefix
                && candidate_stable_prefix_tokens > stable_prefix_budget_tokens
                && !prepared.source.required
            {
                omitted_sources.push(ContextOmittedSource {
                    source: prepared.source,
                    estimated_tokens: item_tokens,
                    inclusion_reason: prepared.inclusion_reason,
                    omission_reason: ContextOmissionReason::StablePrefixBudgetExceeded,
                });
                continue;
            }

            included_items = candidate_items;
            included_sources.push(ContextIncludedSource {
                source: prepared.source,
                tokens: item_tokens,
                reason: prepared.inclusion_reason,
            });
        }

        let sections = build_sections(included_items, &self.config.token_estimator)?;
        let rendered = render_context_sections(&sections);
        let input_tokens = estimate_text(&self.config.token_estimator, &rendered)?;

        Ok(ContextCapsule {
            content: rendered.clone(),
            rendered,
            sections,
            manifest_report: self.manifest_report.clone(),
            token_report: ContextTokenReport {
                input_tokens,
                max_input_tokens: self.config.max_input_tokens,
                stable_prefix_budget_tokens: self.config.stable_prefix_budget_tokens(),
                stable_prefix_budget_ratio_ppm: self.config.stable_prefix_budget_ratio_ppm,
                estimator: self.config.token_estimator.report(),
                included_sources,
                omitted_sources,
            },
        })
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new(ContextBuilderConfig::default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBuilderConfig {
    pub max_input_tokens: u64,
    pub stable_prefix_budget_ratio_ppm: u32,
    pub token_estimator: TokenEstimatorConfig,
}

impl ContextBuilderConfig {
    pub fn new(max_input_tokens: u64) -> Self {
        Self {
            max_input_tokens,
            stable_prefix_budget_ratio_ppm: DEFAULT_STABLE_PREFIX_BUDGET_RATIO_PPM,
            token_estimator: TokenEstimatorConfig::default(),
        }
    }

    pub fn with_token_estimator(
        mut self,
        token_estimator: impl Into<TokenEstimatorConfig>,
    ) -> Self {
        self.token_estimator = token_estimator.into();
        self
    }

    pub const fn with_stable_prefix_budget_ratio_ppm(mut self, ratio_ppm: u32) -> Self {
        self.stable_prefix_budget_ratio_ppm = ratio_ppm;
        self
    }

    pub fn stable_prefix_budget_tokens(&self) -> u64 {
        ((u128::from(self.max_input_tokens) * u128::from(self.stable_prefix_budget_ratio_ppm))
            / u128::from(PPM_SCALE)) as u64
    }
}

impl Default for ContextBuilderConfig {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_INPUT_TOKENS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCapsule {
    pub sections: Vec<ContextSection>,
    pub rendered: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_report: Option<ContextManifestReport>,
    pub token_report: ContextTokenReport,
}

impl ContextCapsule {
    pub fn context_built_payload(&self) -> Value {
        let mut payload = json!({
            "inputTokens": self.token_report.input_tokens,
            "maxInputTokens": self.token_report.max_input_tokens,
            "estimator": self.token_report.estimator,
            "stablePrefixHash": self.stable_prefix_hash(),
            "stablePrefixTokens": self.section_tokens(CachePlacement::StablePrefix),
            "stablePrefixBudgetTokens": self.token_report.stable_prefix_budget_tokens,
            "stablePrefixBudgetRatioPpm": self.token_report.stable_prefix_budget_ratio_ppm,
            "dynamicPreludeTokens": self.section_tokens(CachePlacement::DynamicPrelude),
            "turnSuffixTokens": self.section_tokens(CachePlacement::TurnSuffix),
            "sections": self.sections.iter().map(|section| {
                json!({
                    "placement": section.placement,
                    "tokens": section.tokens,
                    "itemCount": section.items.len(),
                })
            }).collect::<Vec<_>>(),
            "includedSources": self.token_report.included_sources,
            "omittedSources": self.token_report.omitted_sources,
        });
        if let Some(manifest_report) = &self.manifest_report
            && let Value::Object(payload) = &mut payload
        {
            payload.insert("manifest".to_owned(), json!(manifest_report));
        }

        payload
    }

    pub fn rendered_section(&self, placement: CachePlacement) -> Option<String> {
        self.sections
            .iter()
            .find(|section| section.placement == placement)
            .map(render_context_section)
    }

    pub fn stable_prefix_hash(&self) -> String {
        let stable_prefix = self
            .rendered_section(CachePlacement::StablePrefix)
            .unwrap_or_default();
        format!("sha256:{}", sha256_hex(stable_prefix.as_bytes()))
    }

    fn section_tokens(&self, placement: CachePlacement) -> u64 {
        section_tokens(&self.sections, placement)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextManifestReport {
    pub manifest_hash: String,
    pub max_entries: usize,
    pub total_discovered_files: usize,
    pub included_files: usize,
    pub omitted: Vec<ContextManifestOmitted>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextManifestOmitted {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSection {
    pub placement: CachePlacement,
    pub tokens: u64,
    pub items: Vec<ContextSectionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSectionItem {
    pub placement: CachePlacement,
    #[serde(flatten)]
    pub source: ContextSourceRef,
    pub content: String,
    pub tokens: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePlacement {
    StablePrefix,
    DynamicPrelude,
    TurnSuffix,
}

impl CachePlacement {
    const fn order(self) -> u8 {
        match self {
            Self::StablePrefix => 0,
            Self::DynamicPrelude => 1,
            Self::TurnSuffix => 2,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::StablePrefix => "Stable Prefix",
            Self::DynamicPrelude => "Dynamic Prelude",
            Self::TurnSuffix => "Turn Suffix",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextTokenReport {
    pub input_tokens: u64,
    pub max_input_tokens: u64,
    pub stable_prefix_budget_tokens: u64,
    pub stable_prefix_budget_ratio_ppm: u32,
    pub estimator: TokenEstimatorReport,
    pub included_sources: Vec<ContextIncludedSource>,
    pub omitted_sources: Vec<ContextOmittedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextIncludedSource {
    #[serde(flatten)]
    pub source: ContextSourceRef,
    pub tokens: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextOmittedSource {
    #[serde(flatten)]
    pub source: ContextSourceRef,
    pub estimated_tokens: u64,
    pub inclusion_reason: String,
    pub omission_reason: ContextOmissionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOmissionReason {
    TokenBudgetExceeded,
    StablePrefixBudgetExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSourceRef {
    pub kind: ContextItemKind,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItem {
    pub kind: ContextItemKind,
    pub content: String,
    pub reason: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_placement: Option<CachePlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl ContextItem {
    pub fn required(
        kind: ContextItemKind,
        content: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            content: content.into(),
            reason: reason.into(),
            required: true,
            cache_placement: None,
            path: None,
            command_id: None,
            title: None,
        }
    }

    pub fn optional(
        kind: ContextItemKind,
        content: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            required: false,
            ..Self::required(kind, content, reason)
        }
    }

    pub fn user_task(content: impl Into<String>) -> Self {
        Self::required(
            ContextItemKind::UserTask,
            content,
            "current user task for this agent turn",
        )
    }

    pub fn project_rules(content: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::required(ContextItemKind::ProjectRules, content, reason)
    }

    pub fn workspace_manifest(content: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::optional(ContextItemKind::WorkspaceManifest, content, reason)
            .with_cache_placement(CachePlacement::StablePrefix)
    }

    pub fn file(
        path: impl Into<String>,
        content: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::required(ContextItemKind::File, content, reason).with_path(path)
    }

    pub fn tool_result(
        command_id: impl Into<String>,
        content: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::required(ContextItemKind::ToolResult, content, reason).with_command_id(command_id)
    }

    pub fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn with_cache_placement(mut self, cache_placement: CachePlacement) -> Self {
        self.cache_placement = Some(cache_placement);
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_command_id(mut self, command_id: impl Into<String>) -> Self {
        self.command_id = Some(command_id.into());
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextItemKind {
    SystemPolicy,
    ProjectRules,
    UserTask,
    WorkspaceManifest,
    GitStatus,
    GitDiff,
    File,
    ToolResult,
    Plan,
    AcceptanceCriteria,
    PreviousRunSummary,
    Diagnostic,
    Other,
}

impl ContextItemKind {
    fn priority(self) -> u8 {
        match self {
            Self::SystemPolicy => 0,
            Self::ProjectRules => 1,
            Self::UserTask => 2,
            Self::WorkspaceManifest => 3,
            Self::GitStatus => 4,
            Self::GitDiff => 5,
            Self::File => 6,
            Self::ToolResult => 7,
            Self::Plan => 8,
            Self::AcceptanceCriteria => 9,
            Self::PreviousRunSummary => 10,
            Self::Diagnostic => 11,
            Self::Other => 12,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SystemPolicy => "System Policy",
            Self::ProjectRules => "Project Rules",
            Self::UserTask => "User Task",
            Self::WorkspaceManifest => "Workspace Manifest",
            Self::GitStatus => "Git Status",
            Self::GitDiff => "Git Diff",
            Self::File => "File",
            Self::ToolResult => "Tool Result",
            Self::Plan => "Plan",
            Self::AcceptanceCriteria => "Acceptance Criteria",
            Self::PreviousRunSummary => "Previous Run Summary",
            Self::Diagnostic => "Diagnostic",
            Self::Other => "Other",
        }
    }

    fn is_singleton(self) -> bool {
        matches!(
            self,
            Self::SystemPolicy
                | Self::UserTask
                | Self::WorkspaceManifest
                | Self::GitStatus
                | Self::GitDiff
                | Self::Plan
                | Self::AcceptanceCriteria
                | Self::PreviousRunSummary
        )
    }
}

impl CachePlacement {
    pub const fn default_for_kind(kind: ContextItemKind) -> Self {
        match kind {
            ContextItemKind::SystemPolicy
            | ContextItemKind::ProjectRules
            | ContextItemKind::WorkspaceManifest => Self::StablePrefix,
            ContextItemKind::GitStatus | ContextItemKind::GitDiff | ContextItemKind::Diagnostic => {
                Self::DynamicPrelude
            }
            ContextItemKind::UserTask
            | ContextItemKind::File
            | ContextItemKind::ToolResult
            | ContextItemKind::Plan
            | ContextItemKind::AcceptanceCriteria
            | ContextItemKind::PreviousRunSummary
            | ContextItemKind::Other => Self::TurnSuffix,
        }
    }
}

#[derive(Debug, Error)]
pub enum ContextBuildError {
    #[error("max input tokens must be greater than zero")]
    InvalidMaxInputTokens,
    #[error("stable prefix budget ratio ppm must be in 1..=1000000, got {ratio_ppm}")]
    InvalidStablePrefixBudgetRatio { ratio_ppm: u32 },
    #[error("context item reason must not be empty")]
    EmptyReason,
    #[error("context item path must be workspace-relative: {path}")]
    InvalidPath { path: String },
    #[error("context item command id is invalid: {command_id}")]
    InvalidCommandId { command_id: String },
    #[error("context item kind may appear at most once: {kind:?}")]
    DuplicateSingletonItem { kind: ContextItemKind },
    #[error("duplicate context file path: {path}")]
    DuplicateFilePath { path: String },
    #[error("duplicate context command id: {command_id}")]
    DuplicateCommandId { command_id: String },
    #[error(
        "required context exceeds token budget: used {used_tokens}, required {required_tokens}, max {max_input_tokens}"
    )]
    RequiredContextExceedsBudget {
        max_input_tokens: u64,
        used_tokens: u64,
        required_tokens: u64,
        context_source: ContextSourceRef,
    },
    #[error("context token count overflowed")]
    TokenCountOverflow,
    #[error("token estimation failed: {source}")]
    TokenEstimation {
        #[from]
        source: TokenEstimatorError,
    },
}

struct IndexedContextItem<'a> {
    index: usize,
    item: &'a ContextItem,
}

struct PreparedContextItem {
    placement: CachePlacement,
    source: ContextSourceRef,
    content: String,
    inclusion_reason: String,
}

fn prepare_item(item: &ContextItem) -> Result<PreparedContextItem, ContextBuildError> {
    if item.reason.trim().is_empty() {
        return Err(ContextBuildError::EmptyReason);
    }

    let path = item
        .path
        .as_deref()
        .map(normalize_workspace_relative_path)
        .transpose()?;
    let command_id = item
        .command_id
        .as_deref()
        .map(validate_command_id)
        .transpose()?;
    let title = item
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(redact_text);

    Ok(PreparedContextItem {
        placement: item
            .cache_placement
            .unwrap_or_else(|| CachePlacement::default_for_kind(item.kind)),
        source: ContextSourceRef {
            kind: item.kind,
            required: item.required,
            path,
            command_id,
            title,
        },
        content: redact_text(&item.content),
        inclusion_reason: redact_text(item.reason.trim()),
    })
}

fn validate_unique_source(
    source: &ContextSourceRef,
    seen_singletons: &mut HashSet<ContextItemKind>,
    seen_file_paths: &mut HashSet<String>,
    seen_command_ids: &mut HashSet<String>,
) -> Result<(), ContextBuildError> {
    if source.kind.is_singleton() && !seen_singletons.insert(source.kind) {
        return Err(ContextBuildError::DuplicateSingletonItem { kind: source.kind });
    }

    if source.kind == ContextItemKind::File
        && let Some(path) = &source.path
        && !seen_file_paths.insert(path.clone())
    {
        return Err(ContextBuildError::DuplicateFilePath { path: path.clone() });
    }

    if let Some(command_id) = &source.command_id
        && !seen_command_ids.insert(command_id.clone())
    {
        return Err(ContextBuildError::DuplicateCommandId {
            command_id: command_id.clone(),
        });
    }

    Ok(())
}

fn section_item_from_prepared(
    item: &PreparedContextItem,
    token_estimator: &impl TokenEstimator,
) -> Result<ContextSectionItem, ContextBuildError> {
    let mut section_item = ContextSectionItem {
        placement: item.placement,
        source: item.source.clone(),
        content: item.content.clone(),
        tokens: 0,
        reason: item.inclusion_reason.clone(),
    };
    section_item.tokens = estimate_text(token_estimator, &render_context_item(&section_item))?;

    Ok(section_item)
}

fn build_sections(
    items: Vec<ContextSectionItem>,
    token_estimator: &impl TokenEstimator,
) -> Result<Vec<ContextSection>, ContextBuildError> {
    let mut sections: Vec<ContextSection> = Vec::new();

    for item in items {
        if let Some(section) = sections
            .last_mut()
            .filter(|section| section.placement == item.placement)
        {
            section.items.push(item);
            continue;
        }

        sections.push(ContextSection {
            placement: item.placement,
            tokens: 0,
            items: vec![item],
        });
    }

    for section in &mut sections {
        let tokens = estimate_text(token_estimator, &render_context_section(section))?;
        section.tokens = tokens;
    }

    Ok(sections)
}

fn section_tokens(sections: &[ContextSection], placement: CachePlacement) -> u64 {
    sections
        .iter()
        .find(|section| section.placement == placement)
        .map(|section| section.tokens)
        .unwrap_or(0)
}

fn render_context_sections(sections: &[ContextSection]) -> String {
    if sections.is_empty() {
        return String::new();
    }

    let mut rendered = String::new();
    rendered.push_str("# Context Capsule\n");
    rendered.push_str("Renderer: context_capsule.v1\n");

    for section in sections {
        rendered.push('\n');
        rendered.push_str(&render_context_section(section));
    }

    rendered
}

fn render_context_section(section: &ContextSection) -> String {
    let mut rendered = String::new();
    rendered.push_str("## ");
    rendered.push_str(section.placement.label());
    rendered.push('\n');
    rendered.push_str("Placement: ");
    rendered.push_str(cache_placement_name(section.placement));
    rendered.push('\n');

    for item in &section.items {
        rendered.push('\n');
        rendered.push_str(&render_context_item(item));
    }

    rendered
}

fn render_context_item(item: &ContextSectionItem) -> String {
    let mut rendered = String::new();
    rendered.push_str("### ");
    rendered.push_str(
        item.source
            .title
            .as_deref()
            .unwrap_or(item.source.kind.label()),
    );
    rendered.push('\n');
    rendered.push_str("Kind: ");
    rendered.push_str(kind_name(item.source.kind));
    rendered.push('\n');
    rendered.push_str("Placement: ");
    rendered.push_str(cache_placement_name(item.placement));
    rendered.push('\n');
    rendered.push_str("Required: ");
    rendered.push_str(if item.source.required {
        "true"
    } else {
        "false"
    });
    rendered.push('\n');
    rendered.push_str("Reason: ");
    rendered.push_str(&item.reason);
    rendered.push('\n');

    if let Some(path) = &item.source.path {
        rendered.push_str("Path: ");
        rendered.push_str(path);
        rendered.push('\n');
    }
    if let Some(command_id) = &item.source.command_id {
        rendered.push_str("Command-Id: ");
        rendered.push_str(command_id);
        rendered.push('\n');
    }

    rendered.push('\n');
    rendered.push_str(&item.content);
    rendered.push('\n');
    rendered
}

fn cache_placement_name(placement: CachePlacement) -> &'static str {
    match placement {
        CachePlacement::StablePrefix => "stable_prefix",
        CachePlacement::DynamicPrelude => "dynamic_prelude",
        CachePlacement::TurnSuffix => "turn_suffix",
    }
}

fn kind_name(kind: ContextItemKind) -> &'static str {
    match kind {
        ContextItemKind::SystemPolicy => "system_policy",
        ContextItemKind::ProjectRules => "project_rules",
        ContextItemKind::UserTask => "user_task",
        ContextItemKind::WorkspaceManifest => "workspace_manifest",
        ContextItemKind::GitStatus => "git_status",
        ContextItemKind::GitDiff => "git_diff",
        ContextItemKind::File => "file",
        ContextItemKind::ToolResult => "tool_result",
        ContextItemKind::Plan => "plan",
        ContextItemKind::AcceptanceCriteria => "acceptance_criteria",
        ContextItemKind::PreviousRunSummary => "previous_run_summary",
        ContextItemKind::Diagnostic => "diagnostic",
        ContextItemKind::Other => "other",
    }
}

fn estimate_text(
    token_estimator: &impl TokenEstimator,
    text: &str,
) -> Result<u64, ContextBuildError> {
    token_estimator
        .estimate(text)
        .map_err(|source| match source {
            TokenEstimatorError::TokenCountOverflow => ContextBuildError::TokenCountOverflow,
            source => ContextBuildError::TokenEstimation { source },
        })
}

fn normalize_workspace_relative_path(path: &str) -> Result<String, ContextBuildError> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || has_windows_drive_prefix(trimmed)
    {
        return Err(ContextBuildError::InvalidPath {
            path: path.to_owned(),
        });
    }

    let mut parts = Vec::new();
    for part in trimmed.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                return Err(ContextBuildError::InvalidPath {
                    path: path.to_owned(),
                });
            }
            value => parts.push(value),
        }
    }

    if parts.is_empty() {
        return Err(ContextBuildError::InvalidPath {
            path: path.to_owned(),
        });
    }

    Ok(parts.join("/"))
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validate_command_id(command_id: &str) -> Result<String, ContextBuildError> {
    let trimmed = command_id.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Err(ContextBuildError::InvalidCommandId {
            command_id: command_id.to_owned(),
        });
    }

    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        CachePlacement, ContextBuildError, ContextBuilder, ContextBuilderConfig, ContextCapsule,
        ContextItem, ContextItemKind, ContextManifestOmitted, ContextManifestReport,
        ContextOmissionReason, render_context_section,
    };
    use crate::run_log::REDACTED_VALUE;

    #[test]
    fn context_builder_orders_sources_and_reports_tokens() {
        let capsule = ContextBuilder::new(ContextBuilderConfig::new(10_000))
            .with_item(ContextItem::file(
                "src\\lib.rs",
                "pub mod context;",
                "selected file",
            ))
            .with_item(ContextItem::user_task("implement context builder"))
            .with_item(ContextItem::project_rules(
                "keep docs in Chinese",
                "project instructions",
            ))
            .build()
            .expect("context should build");

        let project_rules_index = capsule
            .content
            .find("### Project Rules")
            .expect("project rules should be rendered");
        let user_task_index = capsule
            .content
            .find("### User Task")
            .expect("user task should be rendered");
        let file_index = capsule
            .content
            .find("### File")
            .expect("file should be rendered");

        assert!(project_rules_index < user_task_index);
        assert!(user_task_index < file_index);
        assert_eq!(capsule.rendered, capsule.content);
        assert_eq!(
            capsule.token_report.input_tokens,
            u64::try_from(capsule.rendered.len()).expect("content length should fit u64")
        );
        assert_eq!(capsule.token_report.estimator.name, "utf8_bytes");
        assert!(!capsule.token_report.estimator.exact);
        assert_eq!(capsule.token_report.included_sources.len(), 3);
        assert_eq!(capsule.sections.len(), 2);
        assert_eq!(capsule.sections[0].placement, CachePlacement::StablePrefix);
        assert_eq!(capsule.sections[1].placement, CachePlacement::TurnSuffix);
        assert_eq!(
            capsule.token_report.included_sources[2]
                .source
                .path
                .as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn context_builder_groups_items_by_cache_placement() {
        let capsule = ContextBuilder::new(ContextBuilderConfig::new(10_000))
            .with_item(ContextItem::user_task("fix the parser"))
            .with_item(ContextItem::required(
                ContextItemKind::GitStatus,
                " M src/parser.rs",
                "workspace changes",
            ))
            .with_item(ContextItem::project_rules(
                "keep docs in Chinese",
                "project rules",
            ))
            .build()
            .expect("context should build");

        let placements = capsule
            .sections
            .iter()
            .map(|section| section.placement)
            .collect::<Vec<_>>();
        assert_eq!(
            placements,
            vec![
                CachePlacement::StablePrefix,
                CachePlacement::DynamicPrelude,
                CachePlacement::TurnSuffix,
            ]
        );
        assert_eq!(
            capsule.sections[0].items[0].source.kind,
            ContextItemKind::ProjectRules
        );
        assert_eq!(
            capsule.sections[1].items[0].source.kind,
            ContextItemKind::GitStatus
        );
        assert_eq!(
            capsule.sections[2].items[0].source.kind,
            ContextItemKind::UserTask
        );
    }

    #[test]
    fn context_builder_allows_explicit_cache_placement_override() {
        let capsule = ContextBuilder::new(ContextBuilderConfig::new(10_000))
            .with_item(
                ContextItem::file("src/lib.rs", "pub mod context;", "stable snapshot")
                    .with_cache_placement(CachePlacement::StablePrefix),
            )
            .with_item(ContextItem::user_task("review this file"))
            .build()
            .expect("context should build");

        assert_eq!(capsule.sections[0].placement, CachePlacement::StablePrefix);
        assert_eq!(
            capsule.sections[0].items[0].source.kind,
            ContextItemKind::File
        );
        assert_eq!(
            capsule.sections[0].items[0].source.path.as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn stable_prefix_rendering_survives_turn_suffix_changes() {
        let first = stable_prefix_fixture("first user task");
        let second = stable_prefix_fixture("second user task with different details");

        assert_eq!(
            rendered_section(&first, CachePlacement::StablePrefix),
            rendered_section(&second, CachePlacement::StablePrefix)
        );
        assert_ne!(first.rendered, second.rendered);
    }

    #[test]
    fn context_builder_omits_optional_items_over_budget() {
        let required =
            ContextItem::required(ContextItemKind::SystemPolicy, "short", "needed every turn");
        let optional = ContextItem::optional(
            ContextItemKind::ToolResult,
            "this optional tool result is intentionally too large for the remaining budget",
            "nice to have",
        )
        .with_command_id("search.1");
        let required_only = ContextBuilder::new(ContextBuilderConfig::new(10_000))
            .with_item(required.clone())
            .build()
            .expect("required item should fit");
        let max_input_tokens = required_only.token_report.input_tokens + 1;

        let capsule = ContextBuilder::new(ContextBuilderConfig::new(max_input_tokens))
            .with_item(required)
            .with_item(optional)
            .build()
            .expect("optional item should be omitted");

        assert_eq!(capsule.token_report.included_sources.len(), 1);
        assert_eq!(capsule.token_report.omitted_sources.len(), 1);
        assert_eq!(
            capsule.token_report.omitted_sources[0].omission_reason,
            ContextOmissionReason::TokenBudgetExceeded
        );
        assert_eq!(
            capsule.token_report.omitted_sources[0]
                .source
                .command_id
                .as_deref(),
            Some("search.1")
        );
    }

    #[test]
    fn context_builder_fails_when_required_item_exceeds_budget() {
        let err = ContextBuilder::new(ContextBuilderConfig::new(16))
            .with_item(ContextItem::required(
                ContextItemKind::UserTask,
                "this required task cannot fit",
                "user task",
            ))
            .build()
            .expect_err("required item should exceed budget");

        assert!(matches!(
            err,
            ContextBuildError::RequiredContextExceedsBudget { .. }
        ));
    }

    #[test]
    fn context_builder_rejects_absolute_or_parent_paths() {
        let parent_path_err = ContextBuilder::default()
            .with_item(ContextItem::file("../secret", "content", "unsafe path"))
            .build()
            .expect_err("parent paths should be rejected");
        assert!(matches!(
            parent_path_err,
            ContextBuildError::InvalidPath { .. }
        ));

        let absolute_path_err = ContextBuilder::default()
            .with_item(ContextItem::file(
                "C:\\absolute\\secret",
                "content",
                "unsafe path",
            ))
            .build()
            .expect_err("absolute paths should be rejected");
        assert!(matches!(
            absolute_path_err,
            ContextBuildError::InvalidPath { .. }
        ));
    }

    #[test]
    fn context_builder_rejects_duplicate_singleton_items() {
        let err = ContextBuilder::default()
            .with_item(ContextItem::user_task("first task"))
            .with_item(ContextItem::user_task("second task"))
            .build()
            .expect_err("duplicate singleton should fail");

        assert!(matches!(
            err,
            ContextBuildError::DuplicateSingletonItem {
                kind: ContextItemKind::UserTask
            }
        ));
    }

    #[test]
    fn context_builder_rejects_duplicate_file_paths_after_normalization() {
        let err = ContextBuilder::default()
            .with_item(ContextItem::file("src\\lib.rs", "first", "selected file"))
            .with_item(ContextItem::file("src/lib.rs", "second", "same file"))
            .build()
            .expect_err("duplicate file path should fail");

        assert!(matches!(
            err,
            ContextBuildError::DuplicateFilePath { path } if path == "src/lib.rs"
        ));
    }

    #[test]
    fn context_builder_rejects_duplicate_command_ids() {
        let err = ContextBuilder::default()
            .with_item(ContextItem::tool_result(
                "search.1",
                "first",
                "search result",
            ))
            .with_item(ContextItem::tool_result(
                "search.1",
                "second",
                "same command",
            ))
            .build()
            .expect_err("duplicate command id should fail");

        assert!(matches!(
            err,
            ContextBuildError::DuplicateCommandId { command_id } if command_id == "search.1"
        ));
    }

    #[test]
    fn context_builder_redacts_secret_like_text() {
        let secret = format!("sk-{}", "not-a-real-secret-value-123");
        let capsule = ContextBuilder::default()
            .with_item(ContextItem::tool_result(
                "shell.1",
                format!("stdout contains {secret}"),
                "tool output may include credentials",
            ))
            .build()
            .expect("context should build");

        assert!(!capsule.content.contains(&secret));
        assert!(capsule.content.contains(REDACTED_VALUE));
    }

    #[test]
    fn context_built_payload_matches_token_report_shape() {
        let capsule = ContextBuilder::default()
            .with_manifest_report(ContextManifestReport {
                manifest_hash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
                max_entries: 500,
                total_discovered_files: 3,
                included_files: 2,
                omitted: vec![ContextManifestOmitted {
                    reason: "max_entries_exceeded".to_owned(),
                    count: 1,
                }],
            })
            .with_item(ContextItem::workspace_manifest(
                "Workspace Manifest v0\nManifest-Hash: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
                "stable manifest summary",
            ))
            .with_item(ContextItem::user_task("summarize this repository"))
            .build()
            .expect("context should build");
        let payload = capsule.context_built_payload();

        assert_eq!(payload["inputTokens"], capsule.token_report.input_tokens);
        assert_eq!(payload["maxInputTokens"], 1_000_000);
        assert_eq!(payload["estimator"]["name"], "utf8_bytes");
        assert_eq!(
            payload["includedSources"][0]["kind"],
            serde_json::json!("workspace_manifest")
        );
        assert!(payload["stablePrefixTokens"].as_u64().unwrap_or_default() > 0);
        assert!(payload["turnSuffixTokens"].as_u64().unwrap_or_default() > 0);
        assert_eq!(
            payload["manifest"]["manifestHash"],
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            payload["manifest"]["omitted"][0]["reason"],
            "max_entries_exceeded"
        );
    }

    fn stable_prefix_fixture(user_task: &str) -> ContextCapsule {
        ContextBuilder::new(ContextBuilderConfig::new(10_000))
            .with_item(ContextItem::project_rules(
                "keep documentation in Chinese",
                "project rules",
            ))
            .with_item(ContextItem::user_task(user_task))
            .build()
            .expect("context should build")
    }

    fn rendered_section(capsule: &ContextCapsule, placement: CachePlacement) -> String {
        let section = capsule
            .sections
            .iter()
            .find(|section| section.placement == placement)
            .expect("section should exist");
        render_context_section(section)
    }
}
