use std::{
    collections::BTreeMap,
    fs, io,
    path::{Component, Path, PathBuf},
};

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    cancellation::{CancellationError, CancellationToken},
    hashing::sha256_hex,
};

pub const WORKSPACE_MANIFEST_VERSION: u32 = 1;
pub const DEFAULT_WORKSPACE_MANIFEST_MAX_ENTRIES: usize = 500;

const WORKSPACE_ROOT_PLACEHOLDER: &str = "<workspace>";
const PROLE_CODER_IGNORE_FILE: &str = ".prole-coderignore";
const HARD_EXCLUDED_COMPONENTS: &[&str] = &[
    ".git",
    ".secrets",
    ".secret",
    ".agents",
    ".codex",
    ".prole-coder",
];
const DEFAULT_EXCLUDED_COMPONENTS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    "coverage",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifestConfig {
    pub max_entries: usize,
    pub respect_gitignore: bool,
}

impl WorkspaceManifestConfig {
    pub const fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            respect_gitignore: true,
        }
    }

    pub const fn with_respect_gitignore(mut self, respect_gitignore: bool) -> Self {
        self.respect_gitignore = respect_gitignore;
        self
    }
}

impl Default for WorkspaceManifestConfig {
    fn default() -> Self {
        Self::new(DEFAULT_WORKSPACE_MANIFEST_MAX_ENTRIES)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifest {
    pub manifest_version: u32,
    pub manifest_hash: String,
    pub workspace_root: String,
    pub scan_root: String,
    pub max_entries: usize,
    pub total_discovered_files: usize,
    pub included_files: usize,
    pub total_size_bytes: u64,
    pub entries: Vec<WorkspaceManifestEntry>,
    pub omitted: Vec<WorkspaceManifestOmitted>,
}

impl WorkspaceManifest {
    pub fn summary_markdown(&self) -> String {
        let mut summary = String::new();
        summary.push_str("Workspace Manifest v0\n");
        summary.push_str("Manifest-Hash: ");
        summary.push_str(&self.manifest_hash);
        summary.push('\n');
        summary.push_str("Files: ");
        summary.push_str(&self.included_files.to_string());
        summary.push('/');
        summary.push_str(&self.total_discovered_files.to_string());
        summary.push('\n');
        summary.push_str("Max-Entries: ");
        summary.push_str(&self.max_entries.to_string());
        summary.push('\n');

        if !self.omitted.is_empty() {
            summary.push_str("Omitted:\n");
            for omitted in &self.omitted {
                summary.push_str("- ");
                summary.push_str(omitted.reason.as_str());
                summary.push_str(": ");
                summary.push_str(&omitted.count.to_string());
                summary.push('\n');
            }
        }

        if !self.entries.is_empty() {
            summary.push_str("Entries:\n");
            for entry in &self.entries {
                summary.push_str("- ");
                summary.push_str(&entry.path);
                summary.push_str(" | ");
                summary.push_str(entry.kind.as_str());
                summary.push_str(" | ");
                summary.push_str(&entry.size_bytes.to_string());
                summary.push_str(" bytes | ");
                summary.push_str(entry.risk.as_str());
                summary.push_str(" | ");
                summary.push_str(entry.git.state.as_str());
                if let Some(object_id) = &entry.git.object_id {
                    summary.push(' ');
                    summary.push_str(object_id);
                }
                summary.push('\n');
            }
        }

        summary
    }

    pub fn omitted_by_reason(&self) -> BTreeMap<WorkspaceManifestOmitReason, usize> {
        self.omitted
            .iter()
            .map(|omitted| (omitted.reason, omitted.count))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifestEntry {
    pub path: String,
    pub kind: WorkspaceManifestFileKind,
    pub size_bytes: u64,
    pub sha256: String,
    pub git: WorkspaceManifestGit,
    pub risk: WorkspaceManifestRisk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifestGit {
    pub state: WorkspaceManifestGitState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceManifestOmitReason {
    MaxEntriesExceeded,
}

impl WorkspaceManifestOmitReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxEntriesExceeded => "max_entries_exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifestOmitted {
    pub reason: WorkspaceManifestOmitReason,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceManifestFileKind {
    Rust,
    TypeScript,
    JavaScript,
    Markdown,
    Json,
    Toml,
    Yaml,
    Lockfile,
    Text,
    Other,
}

impl WorkspaceManifestFileKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Lockfile => "lockfile",
            Self::Text => "text",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceManifestRisk {
    Source,
    Test,
    Documentation,
    Configuration,
    Generated,
    Unknown,
}

impl WorkspaceManifestRisk {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Test => "test",
            Self::Documentation => "documentation",
            Self::Configuration => "configuration",
            Self::Generated => "generated",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceManifestGitState {
    Clean,
    Modified,
    Untracked,
    NotRepository,
    Unknown,
}

impl WorkspaceManifestGitState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
            Self::NotRepository => "not_repository",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceManifestError {
    #[error("workspace manifest max_entries must be greater than zero")]
    InvalidMaxEntries,
    #[error("workspace manifest scan root must stay inside workspace: {path}")]
    ScanRootOutsideWorkspace { path: String },
    #[error("workspace manifest path is invalid: {path}")]
    InvalidPath { path: String },
    #[error("workspace manifest I/O failed at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("workspace manifest walk failed: {source}")]
    Walk { source: ignore::Error },
    #[error("workspace manifest serialization failed: {source}")]
    Serialization { source: serde_json::Error },
    #[error("workspace manifest canceled: {source}")]
    Canceled {
        #[from]
        source: CancellationError,
    },
}

pub fn build_workspace_manifest(
    workspace_root: impl AsRef<Path>,
    scan_root: Option<&str>,
    config: WorkspaceManifestConfig,
    cancellation_token: &CancellationToken,
) -> Result<WorkspaceManifest, WorkspaceManifestError> {
    if config.max_entries == 0 {
        return Err(WorkspaceManifestError::InvalidMaxEntries);
    }

    let workspace_root =
        fs::canonicalize(workspace_root.as_ref()).map_err(|source| WorkspaceManifestError::Io {
            path: workspace_root.as_ref().to_path_buf(),
            source,
        })?;
    let scan_root = resolve_scan_root(&workspace_root, scan_root)?;
    let scan_root_label = scan_root
        .strip_prefix(&workspace_root)
        .map(path_to_slash_string)
        .unwrap_or_else(|_| ".".to_owned());
    let scan_root_label = if scan_root_label.is_empty() {
        ".".to_owned()
    } else {
        scan_root_label
    };
    let scan_root_relative = scan_root
        .strip_prefix(&workspace_root)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let git_index = GitIndex::load(&workspace_root);

    let mut walk_builder = WalkBuilder::new(&workspace_root);
    walk_builder
        .hidden(false)
        .parents(false)
        .git_global(false)
        .require_git(false)
        .git_ignore(config.respect_gitignore)
        .git_exclude(config.respect_gitignore)
        .add_custom_ignore_filename(PROLE_CODER_IGNORE_FILE);

    let root_for_filter = workspace_root.clone();
    let scan_root_for_filter = scan_root_relative.clone();
    walk_builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }

        let Ok(relative) = entry.path().strip_prefix(&root_for_filter) else {
            return false;
        };

        is_in_scan_scope(relative, &scan_root_for_filter)
            && !is_hard_excluded(relative)
            && !is_default_excluded(relative)
    });

    let mut entries = Vec::new();
    for entry in walk_builder.build() {
        cancellation_token.check()?;
        let entry = entry.map_err(|source| WorkspaceManifestError::Walk { source })?;
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }

        let relative_path = entry.path().strip_prefix(&workspace_root).map_err(|_| {
            WorkspaceManifestError::ScanRootOutsideWorkspace {
                path: entry.path().display().to_string(),
            }
        })?;
        if !is_in_scan_scope(relative_path, &scan_root_relative) {
            continue;
        }
        if is_hard_excluded(relative_path) || is_default_excluded(relative_path) {
            continue;
        }

        let path = path_to_slash_string(relative_path);
        let bytes = fs::read(entry.path()).map_err(|source| WorkspaceManifestError::Io {
            path: entry.path().to_path_buf(),
            source,
        })?;
        let size_bytes = bytes.len() as u64;
        entries.push(WorkspaceManifestEntry {
            kind: classify_file_kind(&path),
            risk: classify_risk(&path),
            git: git_index.git_for_path(&path),
            path,
            size_bytes,
            sha256: sha256_hex(&bytes),
        });
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let total_discovered_files = entries.len();
    let total_size_bytes = entries.iter().map(|entry| entry.size_bytes).sum::<u64>();
    let omitted_count = total_discovered_files.saturating_sub(config.max_entries);
    if entries.len() > config.max_entries {
        entries.truncate(config.max_entries);
    }
    let included_files = entries.len();
    let omitted = (omitted_count > 0)
        .then_some(WorkspaceManifestOmitted {
            reason: WorkspaceManifestOmitReason::MaxEntriesExceeded,
            count: omitted_count,
        })
        .into_iter()
        .collect::<Vec<_>>();

    let mut manifest = WorkspaceManifest {
        manifest_version: WORKSPACE_MANIFEST_VERSION,
        manifest_hash: String::new(),
        workspace_root: WORKSPACE_ROOT_PLACEHOLDER.to_owned(),
        scan_root: scan_root_label,
        max_entries: config.max_entries,
        total_discovered_files,
        included_files,
        total_size_bytes,
        entries,
        omitted,
    };
    manifest.manifest_hash = compute_manifest_hash(&manifest)?;

    Ok(manifest)
}

fn compute_manifest_hash(manifest: &WorkspaceManifest) -> Result<String, WorkspaceManifestError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ManifestHashInput<'a> {
        manifest_version: u32,
        workspace_root: &'a str,
        scan_root: &'a str,
        max_entries: usize,
        total_discovered_files: usize,
        included_files: usize,
        total_size_bytes: u64,
        entries: &'a [WorkspaceManifestEntry],
        omitted: &'a [WorkspaceManifestOmitted],
    }

    let input = ManifestHashInput {
        manifest_version: manifest.manifest_version,
        workspace_root: &manifest.workspace_root,
        scan_root: &manifest.scan_root,
        max_entries: manifest.max_entries,
        total_discovered_files: manifest.total_discovered_files,
        included_files: manifest.included_files,
        total_size_bytes: manifest.total_size_bytes,
        entries: &manifest.entries,
        omitted: &manifest.omitted,
    };
    let canonical = serde_json::to_vec(&input)
        .map_err(|source| WorkspaceManifestError::Serialization { source })?;

    Ok(format!("sha256:{}", sha256_hex(&canonical)))
}

fn is_in_scan_scope(relative: &Path, scan_root_relative: &Path) -> bool {
    scan_root_relative.as_os_str().is_empty()
        || relative.starts_with(scan_root_relative)
        || scan_root_relative.starts_with(relative)
}

fn resolve_scan_root(
    workspace_root: &Path,
    scan_root: Option<&str>,
) -> Result<PathBuf, WorkspaceManifestError> {
    let Some(scan_root) = scan_root
        .map(str::trim)
        .filter(|scan_root| !scan_root.is_empty())
    else {
        return Ok(workspace_root.to_path_buf());
    };
    let normalized = normalize_workspace_relative_path(scan_root)?;
    let path = workspace_root.join(Path::new(&normalized));
    let canonical = fs::canonicalize(&path).map_err(|source| WorkspaceManifestError::Io {
        path: path.clone(),
        source,
    })?;
    if !canonical.starts_with(workspace_root) {
        return Err(WorkspaceManifestError::ScanRootOutsideWorkspace {
            path: scan_root.to_owned(),
        });
    }

    Ok(canonical)
}

fn normalize_workspace_relative_path(path: &str) -> Result<String, WorkspaceManifestError> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || has_windows_drive_prefix(trimmed)
    {
        return Err(WorkspaceManifestError::InvalidPath {
            path: path.to_owned(),
        });
    }

    let mut parts = Vec::new();
    for part in trimmed.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                return Err(WorkspaceManifestError::InvalidPath {
                    path: path.to_owned(),
                });
            }
            value => parts.push(value),
        }
    }

    if parts.is_empty() {
        return Err(WorkspaceManifestError::InvalidPath {
            path: path.to_owned(),
        });
    }

    Ok(parts.join("/"))
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_hard_excluded(path: &Path) -> bool {
    has_component(path, HARD_EXCLUDED_COMPONENTS)
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_secret_like_file_name)
}

fn is_default_excluded(path: &Path) -> bool {
    has_component(path, DEFAULT_EXCLUDED_COMPONENTS)
}

fn has_component(path: &Path, excluded_components: &[&str]) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => value
            .to_str()
            .is_some_and(|name| excluded_components.contains(&name)),
        _ => false,
    })
}

fn is_secret_like_file_name(name: &str) -> bool {
    matches!(
        name,
        ".env" | ".env.local" | ".env.development" | ".env.production"
    ) || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.eq_ignore_ascii_case("id_rsa")
        || name.eq_ignore_ascii_case("id_ed25519")
}

fn classify_file_kind(path: &str) -> WorkspaceManifestFileKind {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    if matches!(
        file_name,
        "Cargo.lock" | "pnpm-lock.yaml" | "package-lock.json"
    ) {
        return WorkspaceManifestFileKind::Lockfile;
    }

    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "rs" => WorkspaceManifestFileKind::Rust,
        "ts" | "tsx" => WorkspaceManifestFileKind::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => WorkspaceManifestFileKind::JavaScript,
        "md" | "markdown" => WorkspaceManifestFileKind::Markdown,
        "json" => WorkspaceManifestFileKind::Json,
        "toml" => WorkspaceManifestFileKind::Toml,
        "yaml" | "yml" => WorkspaceManifestFileKind::Yaml,
        "txt" => WorkspaceManifestFileKind::Text,
        _ => WorkspaceManifestFileKind::Other,
    }
}

fn classify_risk(path: &str) -> WorkspaceManifestRisk {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    if path.contains("/tests/")
        || path.contains("/test/")
        || file_name.ends_with("_test.rs")
        || file_name.ends_with(".test.ts")
        || file_name.ends_with(".spec.ts")
    {
        return WorkspaceManifestRisk::Test;
    }
    if matches!(
        classify_file_kind(path),
        WorkspaceManifestFileKind::Markdown
    ) {
        return WorkspaceManifestRisk::Documentation;
    }
    if matches!(
        file_name,
        "Cargo.toml"
            | "package.json"
            | "pnpm-workspace.yaml"
            | "tsconfig.json"
            | "tsconfig.base.json"
    ) {
        return WorkspaceManifestRisk::Configuration;
    }
    if file_name.ends_with(".generated.rs")
        || file_name.ends_with(".generated.ts")
        || path.contains("/generated/")
    {
        return WorkspaceManifestRisk::Generated;
    }
    match classify_file_kind(path) {
        WorkspaceManifestFileKind::Rust
        | WorkspaceManifestFileKind::TypeScript
        | WorkspaceManifestFileKind::JavaScript => WorkspaceManifestRisk::Source,
        WorkspaceManifestFileKind::Json
        | WorkspaceManifestFileKind::Toml
        | WorkspaceManifestFileKind::Yaml
        | WorkspaceManifestFileKind::Lockfile => WorkspaceManifestRisk::Configuration,
        WorkspaceManifestFileKind::Text
        | WorkspaceManifestFileKind::Markdown
        | WorkspaceManifestFileKind::Other => WorkspaceManifestRisk::Unknown,
    }
}

fn path_to_slash_string(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        ".".to_owned()
    } else {
        value
    }
}

#[derive(Debug, Default)]
struct GitIndex {
    is_repository: bool,
    states: BTreeMap<String, WorkspaceManifestGitState>,
    object_ids: BTreeMap<String, String>,
}

impl GitIndex {
    fn load(root: &Path) -> Self {
        let is_repository = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !is_repository {
            return Self::default();
        }

        Self {
            is_repository,
            states: load_git_states(root),
            object_ids: load_git_object_ids(root),
        }
    }

    fn git_for_path(&self, path: &str) -> WorkspaceManifestGit {
        if !self.is_repository {
            return WorkspaceManifestGit {
                state: WorkspaceManifestGitState::NotRepository,
                object_id: None,
            };
        }

        WorkspaceManifestGit {
            state: self
                .states
                .get(path)
                .copied()
                .unwrap_or(WorkspaceManifestGitState::Clean),
            object_id: self.object_ids.get(path).cloned(),
        }
    }
}

fn load_git_states(root: &Path) -> BTreeMap<String, WorkspaceManifestGitState> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-z")
        .arg("--untracked-files=all")
        .output();
    let Ok(output) = output else {
        return BTreeMap::new();
    };
    if !output.status.success() {
        return BTreeMap::new();
    }

    parse_git_status_z(&output.stdout)
}

fn parse_git_status_z(bytes: &[u8]) -> BTreeMap<String, WorkspaceManifestGitState> {
    let mut states = BTreeMap::new();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if record.len() < 4 {
            continue;
        }
        let status = &record[..2];
        let path = String::from_utf8_lossy(&record[3..]).replace('\\', "/");
        let state = if status == b"??" {
            WorkspaceManifestGitState::Untracked
        } else {
            WorkspaceManifestGitState::Modified
        };
        states.insert(path, state);
    }

    states
}

fn load_git_object_ids(root: &Path) -> BTreeMap<String, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .arg("-s")
        .output();
    let Ok(output) = output else {
        return BTreeMap::new();
    };
    if !output.status.success() {
        return BTreeMap::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut object_ids = BTreeMap::new();
    for line in stdout.lines() {
        let Some((metadata, path)) = line.split_once('\t') else {
            continue;
        };
        let mut metadata_parts = metadata.split_whitespace();
        let _mode = metadata_parts.next();
        let Some(object_id) = metadata_parts.next() else {
            continue;
        };
        object_ids.insert(path.replace('\\', "/"), object_id.to_owned());
    }

    object_ids
}

#[cfg(test)]
mod tests {
    use super::{
        WorkspaceManifestConfig, WorkspaceManifestFileKind, WorkspaceManifestGitState,
        WorkspaceManifestOmitReason, WorkspaceManifestRisk, build_workspace_manifest,
    };
    use crate::{cancellation::CancellationToken, test_helpers::TestWorkspace};

    #[test]
    fn workspace_manifest_builds_sorted_entries_hash_and_summary() {
        let workspace = TestWorkspace::new("workspace-manifest");
        workspace.write("src/lib.rs", "pub fn answer() -> i32 { 42 }\n");
        workspace.write("README.md", "# Demo\n");
        workspace.write("ignored.md", "ignore me\n");
        workspace.write("target/generated.rs", "ignored target\n");
        workspace.write(".secrets/token.txt", "secret\n");
        workspace.write(".prole-coderignore", "ignored.md\n");

        let manifest = build_workspace_manifest(
            workspace.path(),
            None,
            WorkspaceManifestConfig::default(),
            &CancellationToken::new(),
        )
        .expect("manifest should build");

        let paths = manifest
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec![".prole-coderignore", "README.md", "src/lib.rs"]);
        assert!(manifest.manifest_hash.starts_with("sha256:"));
        assert_eq!(manifest.manifest_hash.len(), "sha256:".len() + 64);
        assert_eq!(
            manifest.entries[1].kind,
            WorkspaceManifestFileKind::Markdown
        );
        assert_eq!(
            manifest.entries[1].risk,
            WorkspaceManifestRisk::Documentation
        );
        assert_eq!(manifest.entries[2].kind, WorkspaceManifestFileKind::Rust);
        assert_eq!(manifest.entries[2].risk, WorkspaceManifestRisk::Source);
        assert_eq!(
            manifest.entries[0].git.state,
            WorkspaceManifestGitState::NotRepository
        );
        assert!(
            !manifest
                .summary_markdown()
                .contains(workspace.path().display().to_string().as_str())
        );

        let repeat = build_workspace_manifest(
            workspace.path(),
            None,
            WorkspaceManifestConfig::default(),
            &CancellationToken::new(),
        )
        .expect("repeat manifest should build");
        assert_eq!(repeat.manifest_hash, manifest.manifest_hash);
    }

    #[test]
    fn workspace_manifest_truncates_with_omitted_reason() {
        let workspace = TestWorkspace::new("workspace-manifest");
        workspace.write("a.txt", "a");
        workspace.write("b.txt", "b");
        workspace.write("c.txt", "c");

        let manifest = build_workspace_manifest(
            workspace.path(),
            None,
            WorkspaceManifestConfig::new(1),
            &CancellationToken::new(),
        )
        .expect("manifest should build");

        assert_eq!(manifest.included_files, 1);
        assert_eq!(manifest.total_discovered_files, 3);
        assert_eq!(manifest.omitted.len(), 1);
        assert_eq!(
            manifest.omitted[0].reason,
            WorkspaceManifestOmitReason::MaxEntriesExceeded
        );
        assert_eq!(manifest.omitted[0].count, 2);
        assert!(
            manifest
                .summary_markdown()
                .contains("max_entries_exceeded: 2")
        );
    }

    #[test]
    fn workspace_manifest_can_scan_subtree() {
        let workspace = TestWorkspace::new("workspace-manifest");
        workspace.write("src/lib.rs", "pub mod lib;\n");
        workspace.write("src/generated.rs", "pub const GENERATED: bool = true;\n");
        workspace.write("README.md", "# Demo\n");
        workspace.write(".gitignore", "src/generated.rs\n");

        let manifest = build_workspace_manifest(
            workspace.path(),
            Some("src"),
            WorkspaceManifestConfig::default(),
            &CancellationToken::new(),
        )
        .expect("manifest should build");

        assert_eq!(manifest.scan_root, "src");
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].path, "src/lib.rs");
    }
}
