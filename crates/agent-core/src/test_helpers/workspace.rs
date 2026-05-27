use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const KEEP_DEMO_WORKSPACE_FLAG: &str = "PROLE_CODER_KEEP_DEMO_WORKSPACE";

static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(1);

pub struct TestWorkspace {
    path: PathBuf,
    path_string: String,
    preserve: bool,
}

impl TestWorkspace {
    pub fn new(label: &str) -> Self {
        Self::new_impl(label, false)
    }

    pub fn with_git(label: &str) -> Self {
        let workspace = Self::new_impl(label, false);
        workspace.git_init();
        workspace
    }

    pub fn with_preserve(label: &str) -> Self {
        Self::new_impl(
            label,
            std::env::var(KEEP_DEMO_WORKSPACE_FLAG).ok().as_deref() == Some("1"),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn path_str(&self) -> &str {
        &self.path_string
    }

    pub fn is_preserved(&self) -> bool {
        self.preserve
    }

    pub fn write(&self, relative: &str, content: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent directory should be created");
        }
        fs::write(path, content).expect("workspace file should be written");
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path.join(relative)).expect("workspace file should read")
    }

    pub fn git_init(&self) {
        self.run_git(["init"]);
        self.run_git(["config", "user.email", "test@example.invalid"]);
        self.run_git(["config", "user.name", "ProleCoder Test"]);
    }

    pub fn git_add(&self, path: &str) {
        self.run_git(["add", path]);
    }

    pub fn git_commit(&self, message: &str) {
        self.run_git(["commit", "-m", message]);
    }

    pub fn git_commit_all(&self, message: &str) {
        self.git_add(".");
        self.git_commit(message);
    }

    pub fn run_git<const N: usize>(&self, args: [&str; N]) -> Output {
        let output = self.run("git", args);
        assert_command_success("git", &output);
        output
    }

    pub fn run<const N: usize>(&self, program: &str, args: [&str; N]) -> Output {
        Command::new(program)
            .args(args)
            .current_dir(&self.path)
            .output()
            .unwrap_or_else(|source| panic!("{program} should run: {source}"))
    }

    fn new_impl(label: &str, preserve: bool) -> Self {
        let id = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed);
        let unique = format!(
            "prole-coder-{}-{}-{}-{}",
            sanitize_label(label),
            std::process::id(),
            id,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("temp workspace should be created");
        let path_string = path.display().to_string();
        Self {
            path,
            path_string,
            preserve,
        }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn sanitize_label(label: &str) -> String {
    let sanitized = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "workspace".to_owned()
    } else {
        sanitized
    }
}

fn assert_command_success(program: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{program} command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
