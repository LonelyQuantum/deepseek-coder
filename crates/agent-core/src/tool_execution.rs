use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::run_log::redact_value;

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SEARCH_MAX_RESULTS: usize = 100;

#[derive(Debug, Clone)]
pub struct WorkspaceToolExecutor {
    root: PathBuf,
}

impl WorkspaceToolExecutor {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ToolExecutionError> {
        let root = fs::canonicalize(root.as_ref()).map_err(|source| ToolExecutionError::Io {
            path: root.as_ref().to_path_buf(),
            source,
        })?;

        if !root.is_dir() {
            return Err(ToolExecutionError::WorkspaceRootNotDirectory { path: root });
        }

        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_file(&self, args: ReadFileArgs) -> Result<ReadFileResult, ToolExecutionError> {
        let path = self.resolve_existing_workspace_path(&args.path)?;
        if !path.is_file() {
            return Err(ToolExecutionError::PathNotFile { path });
        }

        let full_content = fs::read_to_string(&path).map_err(|source| ToolExecutionError::Io {
            path: path.clone(),
            source,
        })?;
        let line_count = count_lines(&full_content);
        let content = select_line_range(
            &full_content,
            line_count,
            args.start_line,
            args.end_line,
            &args.path,
        )?;

        Ok(ReadFileResult {
            status: ToolStatus::Ok,
            summary: format!("Read {}.", args.path),
            error_code: None,
            path: args.path,
            content,
            line_count,
        })
    }

    pub fn search(&self, args: SearchArgs) -> Result<SearchResult, ToolExecutionError> {
        if args.query.trim().is_empty() {
            return Err(ToolExecutionError::InvalidArgument(
                "search query must not be empty".to_owned(),
            ));
        }

        let max_results = args
            .max_results
            .unwrap_or(DEFAULT_SEARCH_MAX_RESULTS)
            .max(1);
        let mut command_args = vec![
            "--json".to_owned(),
            "--fixed-strings".to_owned(),
            "--line-number".to_owned(),
            "--column".to_owned(),
            "--color".to_owned(),
            "never".to_owned(),
            "--glob".to_owned(),
            "!.git/**".to_owned(),
            "--glob".to_owned(),
            "!.secrets/**".to_owned(),
            "--glob".to_owned(),
            "!.secret/**".to_owned(),
            "--glob".to_owned(),
            "!.env".to_owned(),
            "--glob".to_owned(),
            "!.env.*".to_owned(),
            "--glob".to_owned(),
            "!node_modules/**".to_owned(),
            "--glob".to_owned(),
            "!target/**".to_owned(),
        ];
        if !args.case_sensitive.unwrap_or(false) {
            command_args.push("--ignore-case".to_owned());
        }
        command_args.push(args.query);

        if args.paths.is_empty() {
            command_args.push(".".to_owned());
        } else {
            for path in &args.paths {
                let resolved = self.resolve_existing_workspace_path(path)?;
                command_args.push(self.relative_path_string(&resolved)?);
            }
        }

        let output = run_command(
            "rg",
            command_args.iter().map(String::as_str),
            &self.root,
            DEFAULT_COMMAND_TIMEOUT,
        )?;

        if !matches!(output.exit_code, Some(0) | Some(1)) {
            return Err(ToolExecutionError::CommandFailed {
                program: "rg".to_owned(),
                exit_code: output.exit_code,
                stderr: output.stderr,
            });
        }

        let mut matches = Vec::new();
        for line in output.stdout.lines() {
            if matches.len() >= max_results {
                break;
            }

            let event: Value =
                serde_json::from_str(line).map_err(|source| ToolExecutionError::InvalidJson {
                    source,
                    body: line.to_owned(),
                })?;
            if event.get("type").and_then(Value::as_str) != Some("match") {
                continue;
            }

            let data =
                event
                    .get("data")
                    .ok_or_else(|| ToolExecutionError::MalformedToolOutput {
                        detail: "rg match event missing data".to_owned(),
                    })?;
            let path = data
                .pointer("/path/text")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolExecutionError::MalformedToolOutput {
                    detail: "rg match event missing path".to_owned(),
                })?
                .replace('\\', "/")
                .trim_start_matches("./")
                .to_owned();
            let line_number = data
                .get("line_number")
                .and_then(Value::as_u64)
                .ok_or_else(|| ToolExecutionError::MalformedToolOutput {
                    detail: "rg match event missing line_number".to_owned(),
                })?;
            let text = data
                .pointer("/lines/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            let column = data
                .get("submatches")
                .and_then(Value::as_array)
                .and_then(|submatches| submatches.first())
                .and_then(|submatch| submatch.get("start"))
                .and_then(Value::as_u64)
                .map(|start| start + 1)
                .unwrap_or(1);

            matches.push(SearchMatch {
                path,
                line: line_number,
                column,
                text,
            });
        }

        let truncated = matches.len() >= max_results && output.stdout.lines().count() > max_results;
        Ok(SearchResult {
            status: ToolStatus::Ok,
            summary: format!("Found {} matches.", matches.len()),
            error_code: None,
            matches,
            truncated,
            duration_ms: output.duration_ms,
        })
    }

    pub fn apply_patch(
        &self,
        args: ApplyPatchArgs,
    ) -> Result<ApplyPatchResult, ToolExecutionError> {
        let parsed = parse_unified_diff(&args.unified_diff)?;
        if parsed.files.is_empty() {
            return Err(ToolExecutionError::InvalidPatch(
                "patch must contain at least one file".to_owned(),
            ));
        }

        let expected: BTreeSet<String> = args
            .expected_files
            .iter()
            .map(|path| normalize_workspace_relative_path(path))
            .collect::<Result<_, _>>()?;
        let actual: BTreeSet<String> = parsed
            .files
            .iter()
            .map(FilePatch::target_path)
            .collect::<Result<_, _>>()?;
        if expected != actual {
            return Err(ToolExecutionError::PatchFileMismatch {
                expected: expected.into_iter().collect(),
                actual: actual.into_iter().collect(),
            });
        }

        for path in &actual {
            self.resolve_workspace_path(path)?;
        }

        let reverse_patch = parsed.reverse_patch();
        let mut modified_files = Vec::new();
        for file_patch in parsed.files {
            let relative_path = file_patch.target_path()?;
            let path = self.resolve_workspace_path(&relative_path)?;
            let original = if file_patch.old_path.is_none() {
                String::new()
            } else {
                fs::read_to_string(&path).map_err(|source| ToolExecutionError::Io {
                    path: path.clone(),
                    source,
                })?
            };

            let applied = apply_file_patch(&original, &file_patch)?;
            if file_patch.new_path.is_none() {
                if path.exists() {
                    fs::remove_file(&path).map_err(|source| ToolExecutionError::Io {
                        path: path.clone(),
                        source,
                    })?;
                }
            } else {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|source| ToolExecutionError::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                fs::write(&path, applied).map_err(|source| ToolExecutionError::Io {
                    path: path.clone(),
                    source,
                })?;
            }

            modified_files.push(relative_path);
        }

        Ok(ApplyPatchResult {
            status: ToolStatus::Ok,
            summary: format!("Applied patch to {} files.", modified_files.len()),
            error_code: None,
            files: modified_files,
            reverse_patch,
        })
    }

    pub fn shell(&self, args: ShellArgs) -> Result<ShellResult, ToolExecutionError> {
        if args.command.trim().is_empty() {
            return Err(ToolExecutionError::InvalidArgument(
                "shell command must not be empty".to_owned(),
            ));
        }

        let cwd = match args.cwd {
            Some(cwd) => self.resolve_existing_workspace_path(&cwd)?,
            None => self.root.clone(),
        };
        if !cwd.is_dir() {
            return Err(ToolExecutionError::PathNotDirectory { path: cwd });
        }

        let timeout = args
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        let output = run_shell_command(&args.command, &cwd, timeout)?;
        let status = if output.exit_code == Some(0) {
            ToolStatus::Ok
        } else {
            ToolStatus::Failed
        };

        Ok(ShellResult {
            status,
            summary: match status {
                ToolStatus::Ok => "Command completed.".to_owned(),
                ToolStatus::Failed => "Command failed.".to_owned(),
            },
            error_code: (status == ToolStatus::Failed).then(|| "E_COMMAND_FAILED".to_owned()),
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
            duration_ms: output.duration_ms,
        })
    }

    pub fn git_status(&self, args: GitStatusArgs) -> Result<GitStatusResult, ToolExecutionError> {
        let output = if args.porcelain.unwrap_or(true) {
            run_command(
                "git",
                ["status", "--short", "--branch"],
                &self.root,
                DEFAULT_COMMAND_TIMEOUT,
            )?
        } else {
            run_command("git", ["status"], &self.root, DEFAULT_COMMAND_TIMEOUT)?
        };
        if output.exit_code != Some(0) {
            return Err(ToolExecutionError::CommandFailed {
                program: "git status".to_owned(),
                exit_code: output.exit_code,
                stderr: output.stderr,
            });
        }

        let mut lines = output.stdout.lines();
        let branch = lines
            .next()
            .filter(|line| line.starts_with("## "))
            .map(|line| line.trim_start_matches("## ").to_owned());
        let entries = if branch.is_some() {
            lines.map(str::to_owned).collect()
        } else {
            output.stdout.lines().map(str::to_owned).collect()
        };

        Ok(GitStatusResult {
            status: ToolStatus::Ok,
            summary: "Read git status.".to_owned(),
            error_code: None,
            branch,
            entries,
        })
    }

    pub fn git_diff(&self, args: GitDiffArgs) -> Result<GitDiffResult, ToolExecutionError> {
        let mut command_args = vec!["diff".to_owned(), "--no-ext-diff".to_owned()];
        if args.staged.unwrap_or(false) {
            command_args.push("--cached".to_owned());
        }
        command_args.push("--".to_owned());
        for path in &args.paths {
            let resolved = self.resolve_existing_workspace_path(path)?;
            command_args.push(self.relative_path_string(&resolved)?);
        }

        let output = run_command(
            "git",
            command_args.iter().map(String::as_str),
            &self.root,
            DEFAULT_COMMAND_TIMEOUT,
        )?;
        if output.exit_code != Some(0) {
            return Err(ToolExecutionError::CommandFailed {
                program: "git diff".to_owned(),
                exit_code: output.exit_code,
                stderr: output.stderr,
            });
        }

        let files = diff_file_paths(&output.stdout);
        Ok(GitDiffResult {
            status: ToolStatus::Ok,
            summary: format!("Read git diff for {} files.", files.len()),
            error_code: None,
            unified_diff: output.stdout,
            files,
        })
    }

    fn resolve_workspace_path(&self, relative: &str) -> Result<PathBuf, ToolExecutionError> {
        let normalized = normalize_workspace_relative_path(relative)?;
        reject_sensitive_path(&normalized)?;
        let path = self.root.join(Path::new(&normalized));
        let parent = path.parent().unwrap_or(&self.root);
        let canonical_parent =
            fs::canonicalize(parent).map_err(|source| ToolExecutionError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(ToolExecutionError::PathOutsideWorkspace {
                path: relative.to_owned(),
            });
        }

        Ok(path)
    }

    fn resolve_existing_workspace_path(
        &self,
        relative: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let path = self.resolve_workspace_path(relative)?;
        let canonical = fs::canonicalize(&path).map_err(|source| ToolExecutionError::Io {
            path: path.clone(),
            source,
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(ToolExecutionError::PathOutsideWorkspace {
                path: relative.to_owned(),
            });
        }

        Ok(canonical)
    }

    fn relative_path_string(&self, path: &Path) -> Result<String, ToolExecutionError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            ToolExecutionError::PathOutsideWorkspace {
                path: path.display().to_string(),
            }
        })?;
        Ok(path_to_slash_string(relative))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Ok,
    Failed,
}

impl ToolStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileArgs {
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResult {
    pub status: ToolStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub path: String,
    pub content: String,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchArgs {
    pub query: String,
    #[serde(default)]
    pub paths: Vec<String>,
    pub case_sensitive: Option<bool>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    pub line: u64,
    pub column: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub status: ToolStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub matches: Vec<SearchMatch>,
    pub truncated: bool,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPatchArgs {
    pub unified_diff: String,
    pub expected_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPatchResult {
    pub status: ToolStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub files: Vec<String>,
    pub reverse_patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellArgs {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellResult {
    pub status: ToolStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusArgs {
    pub porcelain: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResult {
    pub status: ToolStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub branch: Option<String>,
    pub entries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffArgs {
    pub staged: Option<bool>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffResult {
    pub status: ToolStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub unified_diff: String,
    pub files: Vec<String>,
}

pub fn redacted_tool_result_value<T: Serialize>(result: &T) -> Result<Value, ToolExecutionError> {
    let value = serde_json::to_value(result)
        .map_err(|source| ToolExecutionError::Serialization { source })?;
    Ok(redact_value(value))
}

#[derive(Debug, Error)]
pub enum ToolExecutionError {
    #[error("workspace root is not a directory: {path}")]
    WorkspaceRootNotDirectory { path: PathBuf },
    #[error("path is outside workspace: {path}")]
    PathOutsideWorkspace { path: String },
    #[error("path is blocked because it may contain local or secret data: {path}")]
    SensitivePath { path: String },
    #[error("path is not a file: {path}")]
    PathNotFile { path: PathBuf },
    #[error("path is not a directory: {path}")]
    PathNotDirectory { path: PathBuf },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("invalid line range for `{path}`")]
    InvalidLineRange { path: String },
    #[error("invalid JSON from tool command: {source}; body: {body}")]
    InvalidJson {
        source: serde_json::Error,
        body: String,
    },
    #[error("malformed tool command output: {detail}")]
    MalformedToolOutput { detail: String },
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("patch files do not match expected files; expected {expected:?}, got {actual:?}")]
    PatchFileMismatch {
        expected: Vec<String>,
        actual: Vec<String>,
    },
    #[error("patch hunk mismatch in {path} at line {line}")]
    PatchHunkMismatch { path: String, line: usize },
    #[error("command `{program}` failed with exit code {exit_code:?}: {stderr}")]
    CommandFailed {
        program: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("command `{program}` timed out after {timeout_ms}ms")]
    CommandTimedOut { program: String, timeout_ms: u128 },
    #[error("tool result serialization failed: {source}")]
    Serialization { source: serde_json::Error },
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("I/O error while running `{program}`: {source}")]
    CommandIo { program: String, source: io::Error },
}

#[derive(Debug, Clone)]
struct CommandOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    duration_ms: u128,
}

fn run_shell_command(
    command: &str,
    cwd: &Path,
    timeout: Duration,
) -> Result<CommandOutput, ToolExecutionError> {
    #[cfg(windows)]
    {
        run_command(
            "powershell",
            ["-NoProfile", "-NonInteractive", "-Command", command],
            cwd,
            timeout,
        )
    }

    #[cfg(not(windows))]
    {
        run_command("sh", ["-c", command], cwd, timeout)
    }
}

fn run_command<'a>(
    program: &str,
    args: impl IntoIterator<Item = &'a str>,
    cwd: &Path,
    timeout: Duration,
) -> Result<CommandOutput, ToolExecutionError> {
    let start = Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ToolExecutionError::CommandIo {
            program: program.to_owned(),
            source,
        })?;

    loop {
        if child
            .try_wait()
            .map_err(|source| ToolExecutionError::CommandIo {
                program: program.to_owned(),
                source,
            })?
            .is_some()
        {
            let output =
                child
                    .wait_with_output()
                    .map_err(|source| ToolExecutionError::CommandIo {
                        program: program.to_owned(),
                        source,
                    })?;
            return Ok(CommandOutput {
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                duration_ms: start.elapsed().as_millis(),
            });
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolExecutionError::CommandTimedOut {
                program: program.to_owned(),
                timeout_ms: timeout.as_millis(),
            });
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn normalize_workspace_relative_path(path: &str) -> Result<String, ToolExecutionError> {
    if path.trim().is_empty() {
        return Err(ToolExecutionError::InvalidArgument(
            "path must not be empty".to_owned(),
        ));
    }

    let path = Path::new(path);
    if path.is_absolute() {
        return Err(ToolExecutionError::PathOutsideWorkspace {
            path: path.display().to_string(),
        });
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ToolExecutionError::PathOutsideWorkspace {
                    path: path.display().to_string(),
                });
            }
        }
    }

    if parts.is_empty() {
        return Ok(".".to_owned());
    }

    let normalized = parts.iter().collect::<PathBuf>();
    Ok(path_to_slash_string(&normalized))
}

fn reject_sensitive_path(path: &str) -> Result<(), ToolExecutionError> {
    let path = Path::new(path);
    for component in path.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        let part = part.to_string_lossy();
        if matches!(
            part.as_ref(),
            ".git" | ".secrets" | ".secret" | ".agents" | ".codex"
        ) || part == ".env"
            || part.starts_with(".env.")
        {
            return Err(ToolExecutionError::SensitivePath {
                path: path_to_slash_string(path),
            });
        }
    }

    Ok(())
}

fn path_to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn count_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

fn select_line_range(
    content: &str,
    line_count: usize,
    start_line: Option<usize>,
    end_line: Option<usize>,
    path: &str,
) -> Result<String, ToolExecutionError> {
    let Some(start_line) = start_line else {
        return Ok(content.to_owned());
    };
    let end_line = end_line.unwrap_or(start_line);
    if start_line == 0 || end_line < start_line || end_line > line_count {
        return Err(ToolExecutionError::InvalidLineRange {
            path: path.to_owned(),
        });
    }

    let selected = content
        .split_inclusive('\n')
        .skip(start_line - 1)
        .take(end_line - start_line + 1)
        .collect::<String>();
    Ok(selected)
}

#[derive(Debug, Clone)]
struct ParsedPatch {
    files: Vec<FilePatch>,
}

impl ParsedPatch {
    fn reverse_patch(&self) -> String {
        let mut output = String::new();
        for file in &self.files {
            output.push_str(&format!(
                "--- {}\n+++ {}\n",
                file.format_new_path(),
                file.format_old_path()
            ));
            for hunk in &file.hunks {
                output.push_str(&format!(
                    "@@ -{}{} +{}{} @@{}\n",
                    hunk.new_start,
                    format_count(hunk.new_count),
                    hunk.old_start,
                    format_count(hunk.old_count),
                    hunk.section
                ));
                for line in &hunk.lines {
                    match line {
                        PatchLine::Context(text) => {
                            output.push(' ');
                            output.push_str(text);
                            output.push('\n');
                        }
                        PatchLine::Remove(text) => {
                            output.push('+');
                            output.push_str(text);
                            output.push('\n');
                        }
                        PatchLine::Add(text) => {
                            output.push('-');
                            output.push_str(text);
                            output.push('\n');
                        }
                    }
                }
            }
        }
        output
    }
}

#[derive(Debug, Clone)]
struct FilePatch {
    old_path: Option<String>,
    new_path: Option<String>,
    hunks: Vec<PatchHunk>,
}

impl FilePatch {
    fn target_path(&self) -> Result<String, ToolExecutionError> {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .cloned()
            .ok_or_else(|| ToolExecutionError::InvalidPatch("file patch has no path".to_owned()))
    }

    fn format_old_path(&self) -> String {
        self.old_path
            .as_ref()
            .map(|path| format!("a/{path}"))
            .unwrap_or_else(|| "/dev/null".to_owned())
    }

    fn format_new_path(&self) -> String {
        self.new_path
            .as_ref()
            .map(|path| format!("b/{path}"))
            .unwrap_or_else(|| "/dev/null".to_owned())
    }
}

#[derive(Debug, Clone)]
struct PatchHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    section: String,
    lines: Vec<PatchLine>,
}

#[derive(Debug, Clone)]
enum PatchLine {
    Context(String),
    Remove(String),
    Add(String),
}

fn parse_unified_diff(diff: &str) -> Result<ParsedPatch, ToolExecutionError> {
    let lines: Vec<&str> = diff.lines().collect();
    let mut index = 0;
    let mut files = Vec::new();

    while index < lines.len() {
        let line = lines[index];
        if line.starts_with("diff --git ") || line.starts_with("index ") {
            index += 1;
            continue;
        }
        if !line.starts_with("--- ") {
            return Err(ToolExecutionError::InvalidPatch(format!(
                "expected file header, got `{line}`"
            )));
        }

        let old_path = parse_patch_path(line.trim_start_matches("--- "))?;
        index += 1;
        if index >= lines.len() || !lines[index].starts_with("+++ ") {
            return Err(ToolExecutionError::InvalidPatch(
                "expected new file header".to_owned(),
            ));
        }
        let new_path = parse_patch_path(lines[index].trim_start_matches("+++ "))?;
        index += 1;

        let mut hunks = Vec::new();
        while index < lines.len() {
            let line = lines[index];
            if line.starts_with("--- ") || line.starts_with("diff --git ") {
                break;
            }
            if !line.starts_with("@@ ") {
                return Err(ToolExecutionError::InvalidPatch(format!(
                    "expected hunk header, got `{line}`"
                )));
            }

            let (old_start, old_count, new_start, new_count, section) = parse_hunk_header(line)?;
            index += 1;
            let mut hunk_lines = Vec::new();
            while index < lines.len() {
                let line = lines[index];
                if line.starts_with("@@ ")
                    || line.starts_with("--- ")
                    || line.starts_with("diff --git ")
                {
                    break;
                }
                if line == r"\ No newline at end of file" {
                    index += 1;
                    continue;
                }

                let (marker, text) = line.split_at(1);
                let patch_line = match marker {
                    " " => PatchLine::Context(text.to_owned()),
                    "-" => PatchLine::Remove(text.to_owned()),
                    "+" => PatchLine::Add(text.to_owned()),
                    _ => {
                        return Err(ToolExecutionError::InvalidPatch(format!(
                            "invalid patch line `{line}`"
                        )));
                    }
                };
                hunk_lines.push(patch_line);
                index += 1;
            }

            hunks.push(PatchHunk {
                old_start,
                old_count,
                new_start,
                new_count,
                section,
                lines: hunk_lines,
            });
        }

        files.push(FilePatch {
            old_path,
            new_path,
            hunks,
        });
    }

    Ok(ParsedPatch { files })
}

fn parse_patch_path(path: &str) -> Result<Option<String>, ToolExecutionError> {
    let path = path.split('\t').next().unwrap_or(path);
    if path == "/dev/null" {
        return Ok(None);
    }

    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    Ok(Some(normalize_workspace_relative_path(path)?))
}

fn parse_hunk_header(
    line: &str,
) -> Result<(usize, usize, usize, usize, String), ToolExecutionError> {
    let rest = line
        .strip_prefix("@@ -")
        .ok_or_else(|| ToolExecutionError::InvalidPatch("invalid hunk header".to_owned()))?;
    let (old_part, rest) = rest
        .split_once(" +")
        .ok_or_else(|| ToolExecutionError::InvalidPatch("invalid old hunk range".to_owned()))?;
    let (new_part, section) = rest
        .split_once(" @@")
        .ok_or_else(|| ToolExecutionError::InvalidPatch("invalid new hunk range".to_owned()))?;

    let (old_start, old_count) = parse_hunk_range(old_part)?;
    let (new_start, new_count) = parse_hunk_range(new_part)?;
    Ok((
        old_start,
        old_count,
        new_start,
        new_count,
        section.to_owned(),
    ))
}

fn parse_hunk_range(range: &str) -> Result<(usize, usize), ToolExecutionError> {
    if let Some((start, count)) = range.split_once(',') {
        Ok((
            start.parse().map_err(|_| {
                ToolExecutionError::InvalidPatch(format!("invalid hunk start `{start}`"))
            })?,
            count.parse().map_err(|_| {
                ToolExecutionError::InvalidPatch(format!("invalid hunk count `{count}`"))
            })?,
        ))
    } else {
        Ok((
            range.parse().map_err(|_| {
                ToolExecutionError::InvalidPatch(format!("invalid hunk start `{range}`"))
            })?,
            1,
        ))
    }
}

fn format_count(count: usize) -> String {
    if count == 1 {
        String::new()
    } else {
        format!(",{count}")
    }
}

fn apply_file_patch(original: &str, patch: &FilePatch) -> Result<String, ToolExecutionError> {
    let original_had_trailing_newline = original.ends_with('\n');
    let original_lines: Vec<&str> = original.lines().collect();
    let mut output = Vec::new();
    let mut cursor = 0;

    for hunk in &patch.hunks {
        let hunk_start = hunk.old_start.saturating_sub(1);
        while cursor < hunk_start {
            let line = original_lines.get(cursor).ok_or_else(|| {
                ToolExecutionError::PatchHunkMismatch {
                    path: patch.target_path().unwrap_or_default(),
                    line: cursor + 1,
                }
            })?;
            output.push((*line).to_owned());
            cursor += 1;
        }

        for line in &hunk.lines {
            match line {
                PatchLine::Context(expected) => {
                    let actual = original_lines.get(cursor).ok_or_else(|| {
                        ToolExecutionError::PatchHunkMismatch {
                            path: patch.target_path().unwrap_or_default(),
                            line: cursor + 1,
                        }
                    })?;
                    if actual != expected {
                        return Err(ToolExecutionError::PatchHunkMismatch {
                            path: patch.target_path().unwrap_or_default(),
                            line: cursor + 1,
                        });
                    }
                    output.push(expected.clone());
                    cursor += 1;
                }
                PatchLine::Remove(expected) => {
                    let actual = original_lines.get(cursor).ok_or_else(|| {
                        ToolExecutionError::PatchHunkMismatch {
                            path: patch.target_path().unwrap_or_default(),
                            line: cursor + 1,
                        }
                    })?;
                    if actual != expected {
                        return Err(ToolExecutionError::PatchHunkMismatch {
                            path: patch.target_path().unwrap_or_default(),
                            line: cursor + 1,
                        });
                    }
                    cursor += 1;
                }
                PatchLine::Add(added) => output.push(added.clone()),
            }
        }
    }

    while cursor < original_lines.len() {
        output.push(original_lines[cursor].to_owned());
        cursor += 1;
    }

    let mut content = output.join("\n");
    if original_had_trailing_newline || !content.is_empty() {
        content.push('\n');
    }
    Ok(content)
}

fn diff_file_paths(diff: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            let path = path.split('\t').next().unwrap_or(path);
            files.insert(path.to_owned());
        }
    }
    files.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use super::{
        ApplyPatchArgs, GitDiffArgs, GitStatusArgs, ReadFileArgs, SearchArgs, ShellArgs,
        ShellResult, ToolExecutionError, ToolStatus, WorkspaceToolExecutor,
        redacted_tool_result_value,
    };
    use crate::run_log::REDACTED_VALUE;

    #[test]
    fn read_file_reads_full_file_and_line_range() {
        let workspace = TestWorkspace::new();
        workspace.write("src/main.rs", "fn main() {}\nprintln!(\"hi\");\n");
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        let full = tools
            .read_file(ReadFileArgs {
                path: "src/main.rs".to_owned(),
                start_line: None,
                end_line: None,
            })
            .expect("file should read");
        assert_eq!(full.line_count, 2);
        assert_eq!(full.content, "fn main() {}\nprintln!(\"hi\");\n");

        let line = tools
            .read_file(ReadFileArgs {
                path: "src/main.rs".to_owned(),
                start_line: Some(2),
                end_line: Some(2),
            })
            .expect("line range should read");
        assert_eq!(line.content, "println!(\"hi\");\n");
    }

    #[test]
    fn read_file_rejects_secret_paths_and_parent_traversal() {
        let workspace = TestWorkspace::new();
        workspace.write(".secrets/deepseek-api-key", "secret");
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        assert!(matches!(
            tools.read_file(ReadFileArgs {
                path: ".secrets/deepseek-api-key".to_owned(),
                start_line: None,
                end_line: None,
            }),
            Err(ToolExecutionError::SensitivePath { .. })
        ));
        assert!(matches!(
            tools.read_file(ReadFileArgs {
                path: "../outside".to_owned(),
                start_line: None,
                end_line: None,
            }),
            Err(ToolExecutionError::PathOutsideWorkspace { .. })
        ));
    }

    #[test]
    fn search_finds_text_and_excludes_secret_paths() {
        let workspace = TestWorkspace::new();
        workspace.write("README.md", "hello visible\n");
        workspace.write(".secrets/token.txt", "hello hidden\n");
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        let result = tools
            .search(SearchArgs {
                query: "hello".to_owned(),
                paths: Vec::new(),
                case_sensitive: Some(true),
                max_results: Some(10),
            })
            .expect("search should run");

        assert_eq!(result.status, ToolStatus::Ok);
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].path, "README.md");
    }

    #[test]
    fn apply_patch_modifies_expected_files_and_returns_reverse_patch() {
        let workspace = TestWorkspace::new();
        workspace.write("README.md", "old\n");
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        let result = tools
            .apply_patch(ApplyPatchArgs {
                unified_diff: concat!(
                    "--- a/README.md\n",
                    "+++ b/README.md\n",
                    "@@ -1 +1 @@\n",
                    "-old\n",
                    "+new\n",
                )
                .to_owned(),
                expected_files: vec!["README.md".to_owned()],
            })
            .expect("patch should apply");

        assert_eq!(result.files, vec!["README.md"]);
        assert_eq!(workspace.read("README.md"), "new\n");
        assert!(result.reverse_patch.contains("-new"));
        assert!(result.reverse_patch.contains("+old"));
    }

    #[test]
    fn apply_patch_rejects_unexpected_files() {
        let workspace = TestWorkspace::new();
        workspace.write("README.md", "old\n");
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        let error = tools
            .apply_patch(ApplyPatchArgs {
                unified_diff: concat!(
                    "--- a/README.md\n",
                    "+++ b/README.md\n",
                    "@@ -1 +1 @@\n",
                    "-old\n",
                    "+new\n",
                )
                .to_owned(),
                expected_files: vec!["src/lib.rs".to_owned()],
            })
            .expect_err("file mismatch should fail");

        assert!(matches!(
            error,
            ToolExecutionError::PatchFileMismatch { .. }
        ));
    }

    #[test]
    fn shell_runs_non_interactive_command() {
        let workspace = TestWorkspace::new();
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        #[cfg(windows)]
        let command = "Write-Output hello";
        #[cfg(not(windows))]
        let command = "printf hello";

        let result = tools
            .shell(ShellArgs {
                command: command.to_owned(),
                cwd: None,
                timeout_ms: Some(10_000),
            })
            .expect("shell should run");

        assert_eq!(result.status, ToolStatus::Ok);
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn redacted_tool_result_value_redacts_shell_output_for_logs() {
        let secret = format!("sk-{}", "not-a-real-tool-output-secret-123");
        let result = ShellResult {
            status: ToolStatus::Ok,
            summary: "Command completed.".to_owned(),
            error_code: None,
            exit_code: Some(0),
            stdout: format!("stdout contains {secret}"),
            stderr: format!("stderr contains {secret}"),
            duration_ms: 1,
        };

        let value =
            redacted_tool_result_value(&result).expect("tool result should serialize and redact");

        assert_eq!(value["stdout"], format!("stdout contains {REDACTED_VALUE}"));
        assert_eq!(value["stderr"], format!("stderr contains {REDACTED_VALUE}"));
        assert!(!value.to_string().contains(&secret));
    }

    #[test]
    fn git_status_and_diff_read_repository_state() {
        let workspace = TestWorkspace::new();
        workspace.git_init();
        workspace.write("README.md", "hello\n");
        let tools = WorkspaceToolExecutor::new(workspace.path()).expect("workspace should open");

        let status = tools
            .git_status(GitStatusArgs {
                porcelain: Some(true),
            })
            .expect("status should run");
        assert!(
            status
                .entries
                .iter()
                .any(|entry| entry.contains("README.md"))
        );

        workspace.git_add("README.md");
        let diff = tools
            .git_diff(GitDiffArgs {
                staged: Some(true),
                paths: vec!["README.md".to_owned()],
            })
            .expect("diff should run");
        assert!(diff.unified_diff.contains("+hello"));
        assert_eq!(diff.files, vec!["README.md"]);
    }

    struct TestWorkspace {
        path: std::path::PathBuf,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let unique = format!(
                "deepseek-coder-agent-core-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock should be after epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp workspace should be created");
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent should be created");
            }
            fs::write(path, content).expect("file should be written");
        }

        fn read(&self, relative: &str) -> String {
            fs::read_to_string(self.path.join(relative)).expect("file should read")
        }

        fn git_init(&self) {
            self.run_git(["init"]);
            self.run_git(["config", "user.email", "test@example.invalid"]);
            self.run_git(["config", "user.name", "DeepSeek Coder Test"]);
        }

        fn git_add(&self, path: &str) {
            self.run_git(["add", path]);
        }

        fn run_git<const N: usize>(&self, args: [&str; N]) {
            let output = Command::new("git")
                .args(args)
                .current_dir(&self.path)
                .output()
                .expect("git should run");
            assert!(
                output.status.success(),
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
