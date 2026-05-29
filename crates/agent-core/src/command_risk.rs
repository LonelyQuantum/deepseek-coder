use crate::approval::RiskLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRiskClassification {
    pub risk: RiskLevel,
    pub reasons: Vec<CommandRiskReason>,
}

impl CommandRiskClassification {
    fn new() -> Self {
        Self {
            risk: RiskLevel::Exec,
            reasons: Vec::new(),
        }
    }

    fn record(&mut self, risk: RiskLevel, reason: CommandRiskReason) {
        self.risk = higher_risk(self.risk, risk);
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }

    pub fn reason_summaries(&self) -> Vec<String> {
        self.reasons
            .iter()
            .map(|reason| reason.summary().to_owned())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandRiskReason {
    DependencyInstall,
    NetworkAccess,
    RemoteGit,
    Publish,
    FileDeletion,
    DestructiveGit,
}

impl CommandRiskReason {
    pub const fn summary(self) -> &'static str {
        match self {
            Self::DependencyInstall => "dependency install/update",
            Self::NetworkAccess => "network access",
            Self::RemoteGit => "remote git operation",
            Self::Publish => "publish/release command",
            Self::FileDeletion => "file deletion",
            Self::DestructiveGit => "destructive git operation",
        }
    }
}

pub fn classify_shell_command(command: &str) -> CommandRiskClassification {
    classify_shell_command_with_depth(command, 0)
}

fn classify_shell_command_with_depth(command: &str, depth: usize) -> CommandRiskClassification {
    let mut classification = CommandRiskClassification::new();
    if depth > 8 {
        return classification;
    }

    for segment in shell_segments(command) {
        classify_segment(&segment, &mut classification, depth);
    }
    for subcommand in shell_subcommands(command) {
        let nested = classify_shell_command_with_depth(&subcommand, depth + 1);
        merge_classification(&mut classification, nested);
    }
    classification
}

fn classify_segment(
    words: &[String],
    classification: &mut CommandRiskClassification,
    depth: usize,
) {
    if depth > 8 {
        return;
    }

    let words = skip_assignment_prefixes(words);
    let Some((executable, args)) = words.split_first() else {
        return;
    };
    let executable = normalize_executable(executable);

    if classify_wrapper(&executable, args, classification, depth) {
        return;
    }

    if NETWORK_EXECUTABLES.contains(&executable.as_str()) {
        classification.record(RiskLevel::Network, CommandRiskReason::NetworkAccess);
    }

    if DELETE_EXECUTABLES.contains(&executable.as_str()) {
        classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
    }

    match executable.as_str() {
        "git" => classify_git(args, classification),
        "npm" | "pnpm" | "yarn" | "bun" => {
            classify_javascript_package_manager(&executable, args, classification);
        }
        "npx" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "cargo" => classify_cargo(args, classification),
        "rustup" => classify_rustup(args, classification),
        "pip" | "pip3" | "pipx" => classify_pip(args, classification),
        "python" | "python3" | "py" => classify_python(args, classification),
        "uv" => classify_uv(args, classification),
        "poetry" | "pipenv" | "conda" | "mamba" | "composer" | "bundle" | "gem" => {
            classify_dependency_subcommands(args, classification);
        }
        "go" => classify_go(args, classification),
        "dotnet" => classify_dotnet(args, classification),
        "nuget" => classify_nuget(args, classification),
        "apt" | "apt-get" | "dnf" | "yum" | "apk" | "brew" | "winget" | "choco" | "scoop"
        | "zypper" | "snap" | "flatpak" => {
            classify_system_package_manager(args, classification);
        }
        "pacman" => classify_pacman(args, classification),
        "twine" => classify_twine(args, classification),
        "gh" | "hub" => classify_github_cli(args, classification),
        "docker" | "podman" => classify_container_cli(args, classification),
        "install-module" | "install-package" | "install-script" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "publish-module" | "publish-script" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "uninstall-module" | "uninstall-package" | "uninstall-script" | "clear-content" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        "find" if has_arg(args, "-delete") => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_wrapper(
    executable: &str,
    args: &[String],
    classification: &mut CommandRiskClassification,
    depth: usize,
) -> bool {
    match executable {
        "sudo" | "doas" | "command" | "time" | "nohup" | "nice" => {
            classify_segment(skip_wrapper_options(args), classification, depth + 1);
            true
        }
        "env" => {
            classify_segment(skip_env_args(args), classification, depth + 1);
            true
        }
        "cmd" => {
            if let Some(command) = command_after_flag(args, &["/c", "/k"]) {
                classify_shell_command_into(command, classification, depth);
            }
            true
        }
        "powershell" | "pwsh" | "powershell_ise" | "sh" | "bash" | "zsh" | "fish" => {
            if let Some(command) = command_after_flag(args, &["-command", "-c"]) {
                classify_shell_command_into(command, classification, depth);
            }
            true
        }
        _ => false,
    }
}

fn classify_shell_command_into(
    command: String,
    classification: &mut CommandRiskClassification,
    depth: usize,
) {
    let nested = classify_shell_command_with_depth(&command, depth + 1);
    merge_classification(classification, nested);
}

fn merge_classification(
    classification: &mut CommandRiskClassification,
    nested: CommandRiskClassification,
) {
    classification.risk = higher_risk(classification.risk, nested.risk);
    for reason in nested.reasons {
        if !classification.reasons.contains(&reason) {
            classification.reasons.push(reason);
        }
    }
}

fn classify_git(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, GIT_OPTIONS_WITH_VALUES, true) else {
        return;
    };

    match command.as_str() {
        "clone" | "fetch" | "pull" | "push" | "ls-remote" | "remote" => {
            classification.record(RiskLevel::Network, CommandRiskReason::RemoteGit);
        }
        "submodule"
            if subcommand_after(args, "submodule").is_some_and(|subcommand| {
                matches!(subcommand.as_str(), "update" | "init" | "sync")
            }) =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::RemoteGit);
        }
        "reset" if has_arg(args, "--hard") => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::DestructiveGit);
        }
        "clean" | "rm" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::DestructiveGit);
        }
        "checkout" if has_any_arg(args, &["-f", "--force"]) => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::DestructiveGit);
        }
        "branch" if has_any_arg(args, &["-D", "--delete", "-d"]) => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::DestructiveGit);
        }
        "tag" if has_any_arg(args, &["-d", "--delete"]) => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::DestructiveGit);
        }
        _ => {}
    }

    if command == "push" && has_any_arg(args, &["-f", "--force", "--force-with-lease", "--delete"])
    {
        classification.record(RiskLevel::Destructive, CommandRiskReason::DestructiveGit);
    }
}

fn classify_javascript_package_manager(
    executable: &str,
    args: &[String],
    classification: &mut CommandRiskClassification,
) {
    let Some(command) = first_subcommand(args, JS_OPTIONS_WITH_VALUES, false) else {
        return;
    };

    if executable == "yarn"
        && command == "npm"
        && subcommand_after(args, "npm").is_some_and(|subcommand| subcommand == "publish")
    {
        classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        return;
    }

    match command.as_str() {
        "install" | "i" | "ci" | "add" | "update" | "upgrade" | "up" | "dlx" | "create" | "x" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "publish" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "unpublish" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::Publish);
        }
        "remove" | "rm" | "uninstall" | "unlink" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_cargo(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, CARGO_OPTIONS_WITH_VALUES, true) else {
        return;
    };

    match command.as_str() {
        "install" | "fetch" | "update" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "publish" | "release" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "uninstall" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_rustup(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "install" | "update" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "toolchain"
            if subcommand_after(args, "toolchain")
                .is_some_and(|subcommand| matches!(subcommand.as_str(), "install" | "add")) =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "component"
            if subcommand_after(args, "component")
                .is_some_and(|subcommand| subcommand == "add") =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        _ => {}
    }
}

fn classify_pip(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, PIP_OPTIONS_WITH_VALUES, false) else {
        return;
    };

    match command.as_str() {
        "install" | "download" | "wheel" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "uninstall" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_python(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(module_index) = args.iter().position(|arg| normalize_arg(arg) == "-m") else {
        return;
    };
    let Some(module) = args.get(module_index + 1).map(|arg| normalize_arg(arg)) else {
        return;
    };

    match module.as_str() {
        "pip" | "pip3" => classify_pip(&args[module_index + 2..], classification),
        "twine" => classify_twine(&args[module_index + 2..], classification),
        _ => {}
    }
}

fn classify_uv(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "add" | "sync" | "lock" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "pip"
            if subcommand_after(args, "pip").is_some_and(|subcommand| {
                matches!(subcommand.as_str(), "install" | "sync" | "compile")
            }) =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "tool"
            if subcommand_after(args, "tool").is_some_and(|subcommand| subcommand == "install") =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "publish" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "remove" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_dependency_subcommands(
    args: &[String],
    classification: &mut CommandRiskClassification,
) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "install" | "add" | "update" | "upgrade" | "require" | "restore" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "publish" | "push" | "release" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "remove" | "uninstall" | "delete" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_go(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "get" | "install" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "mod"
            if subcommand_after(args, "mod").is_some_and(|subcommand| subcommand == "download") =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        _ => {}
    }
}

fn classify_dotnet(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, DOTNET_OPTIONS_WITH_VALUES, false) else {
        return;
    };

    match command.as_str() {
        "restore" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "add"
            if subcommand_after(args, "add").is_some_and(|subcommand| subcommand == "package") =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "remove"
            if subcommand_after(args, "remove")
                .is_some_and(|subcommand| subcommand == "package") =>
        {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        "nuget"
            if subcommand_after(args, "nuget").is_some_and(|subcommand| subcommand == "push") =>
        {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        _ => {}
    }
}

fn classify_nuget(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "install" | "restore" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "push" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "delete" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::Publish);
        }
        _ => {}
    }
}

fn classify_system_package_manager(
    args: &[String],
    classification: &mut CommandRiskClassification,
) {
    let Some(command) = first_subcommand(args, SYSTEM_OPTIONS_WITH_VALUES, false) else {
        return;
    };

    match command.as_str() {
        "install" | "update" | "upgrade" | "dist-upgrade" | "add" | "sync" | "-s" => {
            classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
        }
        "remove" | "uninstall" | "purge" | "erase" | "autoremove" | "delete" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn classify_pacman(args: &[String], classification: &mut CommandRiskClassification) {
    if has_any_arg(args, &["-S", "--sync", "-Sy", "-Syu"]) {
        classification.record(RiskLevel::Network, CommandRiskReason::DependencyInstall);
    }
    if has_any_arg(args, &["-R", "--remove", "-Rs", "-Rns"]) {
        classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
    }
}

fn classify_twine(args: &[String], classification: &mut CommandRiskClassification) {
    if first_subcommand(args, &[], false).is_some_and(|command| command == "upload") {
        classification.record(RiskLevel::Network, CommandRiskReason::Publish);
    }
}

fn classify_github_cli(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "repo" | "pr" | "issue" | "api" | "auth" => {
            classification.record(RiskLevel::Network, CommandRiskReason::NetworkAccess);
        }
        "release" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
            if subcommand_after(args, "release")
                .is_some_and(|subcommand| matches!(subcommand.as_str(), "delete" | "delete-asset"))
            {
                classification.record(RiskLevel::Destructive, CommandRiskReason::Publish);
            }
        }
        _ => {}
    }
}

fn classify_container_cli(args: &[String], classification: &mut CommandRiskClassification) {
    let Some(command) = first_subcommand(args, &[], false) else {
        return;
    };

    match command.as_str() {
        "pull" | "login" => {
            classification.record(RiskLevel::Network, CommandRiskReason::NetworkAccess);
        }
        "push" => {
            classification.record(RiskLevel::Network, CommandRiskReason::Publish);
        }
        "rm" | "rmi" => {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        "volume"
            if subcommand_after(args, "volume").is_some_and(|subcommand| subcommand == "rm") =>
        {
            classification.record(RiskLevel::Destructive, CommandRiskReason::FileDeletion);
        }
        _ => {}
    }
}

fn shell_segments(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = QuoteState::None;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                } else {
                    word.push(ch);
                }
            }
            QuoteState::Double => match ch {
                '"' => quote = QuoteState::None,
                '\\' | '`' => {
                    if let Some(next) = chars.next() {
                        word.push(next);
                    } else {
                        word.push(ch);
                    }
                }
                _ => word.push(ch),
            },
            QuoteState::None => match ch {
                '\'' => quote = QuoteState::Single,
                '"' => quote = QuoteState::Double,
                '\\' | '`' => {
                    if let Some(next) = chars.next() {
                        word.push(next);
                    } else {
                        word.push(ch);
                    }
                }
                ' ' | '\t' | '\r' => flush_word(&mut word, &mut words),
                '\n' | ';' | '|' | '&' | '(' | ')' => {
                    flush_word(&mut word, &mut words);
                    flush_segment(&mut words, &mut segments);
                    if (ch == '|' || ch == '&') && chars.peek().is_some_and(|next| *next == ch) {
                        chars.next();
                    }
                }
                _ => word.push(ch),
            },
        }
    }

    flush_word(&mut word, &mut words);
    flush_segment(&mut words, &mut segments);
    segments
}

fn shell_subcommands(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut subcommands = Vec::new();
    let mut quote = QuoteState::None;
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                index += 1;
            }
            QuoteState::Double => match ch {
                '"' => {
                    quote = QuoteState::None;
                    index += 1;
                }
                '\\' => index += 2,
                '$' if chars.get(index + 1).is_some_and(|next| *next == '(') => {
                    if let Some((subcommand, next_index)) =
                        collect_dollar_subcommand(&chars, index + 2)
                    {
                        push_subcommand(&mut subcommands, subcommand);
                        index = next_index;
                    } else {
                        index += 1;
                    }
                }
                _ => index += 1,
            },
            QuoteState::None => match ch {
                '\'' => {
                    quote = QuoteState::Single;
                    index += 1;
                }
                '"' => {
                    quote = QuoteState::Double;
                    index += 1;
                }
                '\\' => index += 2,
                '$' if chars.get(index + 1).is_some_and(|next| *next == '(') => {
                    if let Some((subcommand, next_index)) =
                        collect_dollar_subcommand(&chars, index + 2)
                    {
                        push_subcommand(&mut subcommands, subcommand);
                        index = next_index;
                    } else {
                        index += 1;
                    }
                }
                '`' => {
                    if let Some((subcommand, next_index)) =
                        collect_backtick_subcommand(&chars, index + 1)
                    {
                        push_subcommand(&mut subcommands, subcommand);
                        index = next_index;
                    } else {
                        index += 1;
                    }
                }
                _ => index += 1,
            },
        }
    }

    subcommands
}

fn collect_dollar_subcommand(chars: &[char], mut index: usize) -> Option<(String, usize)> {
    let mut subcommand = String::new();
    let mut quote = QuoteState::None;
    let mut depth = 1usize;

    while index < chars.len() {
        let ch = chars[index];
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                subcommand.push(ch);
                index += 1;
            }
            QuoteState::Double => match ch {
                '"' => {
                    quote = QuoteState::None;
                    subcommand.push(ch);
                    index += 1;
                }
                '\\' => {
                    subcommand.push(ch);
                    index += 1;
                    if let Some(next) = chars.get(index) {
                        subcommand.push(*next);
                        index += 1;
                    }
                }
                _ => {
                    subcommand.push(ch);
                    index += 1;
                }
            },
            QuoteState::None => match ch {
                '\'' => {
                    quote = QuoteState::Single;
                    subcommand.push(ch);
                    index += 1;
                }
                '"' => {
                    quote = QuoteState::Double;
                    subcommand.push(ch);
                    index += 1;
                }
                '\\' => {
                    subcommand.push(ch);
                    index += 1;
                    if let Some(next) = chars.get(index) {
                        subcommand.push(*next);
                        index += 1;
                    }
                }
                '(' => {
                    depth += 1;
                    subcommand.push(ch);
                    index += 1;
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((subcommand, index + 1));
                    }
                    subcommand.push(ch);
                    index += 1;
                }
                _ => {
                    subcommand.push(ch);
                    index += 1;
                }
            },
        }
    }

    None
}

fn collect_backtick_subcommand(chars: &[char], mut index: usize) -> Option<(String, usize)> {
    let mut subcommand = String::new();

    while index < chars.len() {
        let ch = chars[index];
        if ch == '`' {
            return Some((subcommand, index + 1));
        }
        if ch == '\\' {
            subcommand.push(ch);
            index += 1;
            if let Some(next) = chars.get(index) {
                subcommand.push(*next);
                index += 1;
            }
            continue;
        }
        subcommand.push(ch);
        index += 1;
    }

    None
}

fn push_subcommand(subcommands: &mut Vec<String>, subcommand: String) {
    if !subcommand.trim().is_empty() {
        subcommands.push(subcommand);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    None,
    Single,
    Double,
}

fn flush_word(word: &mut String, words: &mut Vec<String>) {
    if !word.is_empty() {
        words.push(std::mem::take(word));
    }
}

fn flush_segment(words: &mut Vec<String>, segments: &mut Vec<Vec<String>>) {
    if !words.is_empty() {
        segments.push(std::mem::take(words));
    }
}

fn skip_assignment_prefixes(words: &[String]) -> &[String] {
    let mut index = 0;
    while words
        .get(index)
        .is_some_and(|word| is_environment_assignment(word))
    {
        index += 1;
    }
    &words[index..]
}

fn skip_wrapper_options(args: &[String]) -> &[String] {
    let mut index = 0;
    while args
        .get(index)
        .is_some_and(|arg| normalize_arg(arg).starts_with('-'))
    {
        index += 1;
    }
    &args[index..]
}

fn skip_env_args(args: &[String]) -> &[String] {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        let normalized = normalize_arg(arg);
        if is_environment_assignment(arg) {
            index += 1;
        } else if matches!(normalized.as_str(), "-u" | "--unset" | "-0" | "-i") {
            index += 1;
            if matches!(normalized.as_str(), "-u" | "--unset") {
                index += 1;
            }
        } else if normalized.starts_with('-') {
            index += 1;
        } else {
            break;
        }
    }
    &args[index..]
}

fn command_after_flag(args: &[String], flags: &[&str]) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        let current = normalize_arg(&args[index]);
        if flags.contains(&current.as_str()) {
            return Some(args[index + 1..].join(" "));
        }
        index += 1;
    }
    None
}

fn first_subcommand(
    args: &[String],
    options_with_values: &[&str],
    skip_plus_toolchain: bool,
) -> Option<String> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        let normalized = normalize_arg(arg);
        if normalized == "--" {
            index += 1;
            continue;
        }
        if skip_plus_toolchain && normalized.starts_with('+') {
            index += 1;
            continue;
        }
        if option_takes_value(&normalized, options_with_values) {
            index += 2;
            continue;
        }
        if normalized.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(normalized);
    }
    None
}

fn subcommand_after(args: &[String], command: &str) -> Option<String> {
    let command = command.to_ascii_lowercase();
    let index = args.iter().position(|arg| normalize_arg(arg) == command)?;
    args[index + 1..]
        .iter()
        .map(|arg| normalize_arg(arg))
        .find(|arg| !arg.starts_with('-'))
}

fn option_takes_value(arg: &str, options_with_values: &[&str]) -> bool {
    options_with_values.iter().any(|option| {
        arg == *option
            || option
                .strip_prefix("--")
                .is_some_and(|long| arg.starts_with(&format!("--{long}=")))
    })
}

fn has_arg(args: &[String], expected: &str) -> bool {
    args.iter()
        .map(|arg| normalize_arg(arg))
        .any(|arg| arg == expected.to_ascii_lowercase())
}

fn has_any_arg(args: &[String], expected: &[&str]) -> bool {
    args.iter().map(|arg| normalize_arg(arg)).any(|arg| {
        expected
            .iter()
            .any(|expected| arg == expected.to_ascii_lowercase())
    })
}

fn normalize_executable(word: &str) -> String {
    let basename = word
        .trim_start_matches('&')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(word);
    let mut normalized = basename.to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".com"] {
        if normalized.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }
    normalized
}

fn normalize_arg(word: &str) -> String {
    word.to_ascii_lowercase()
}

fn is_environment_assignment(word: &str) -> bool {
    let Some((name, value)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !value.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && !name.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn higher_risk(left: RiskLevel, right: RiskLevel) -> RiskLevel {
    if risk_rank(right) > risk_rank(left) {
        right
    } else {
        left
    }
}

fn risk_rank(risk: RiskLevel) -> u8 {
    match risk {
        RiskLevel::Read => 0,
        RiskLevel::Write => 1,
        RiskLevel::Exec => 2,
        RiskLevel::Network => 3,
        RiskLevel::Destructive => 4,
    }
}

const NETWORK_EXECUTABLES: &[&str] = &[
    "aria2c",
    "curl",
    "ftp",
    "invoke-restmethod",
    "invoke-webrequest",
    "irm",
    "iwr",
    "nc",
    "netcat",
    "rsync",
    "scp",
    "sftp",
    "ssh",
    "start-bitstransfer",
    "telnet",
    "wget",
];

const DELETE_EXECUTABLES: &[&str] = &[
    "del",
    "erase",
    "rd",
    "remove-item",
    "ri",
    "rm",
    "rmdir",
    "unlink",
];

const GIT_OPTIONS_WITH_VALUES: &[&str] = &["-c", "--git-dir", "--work-tree"];
const JS_OPTIONS_WITH_VALUES: &[&str] = &[
    "--prefix",
    "--cache",
    "--registry",
    "--userconfig",
    "--workspace",
    "-w",
    "-c",
];
const CARGO_OPTIONS_WITH_VALUES: &[&str] = &[
    "--config",
    "--manifest-path",
    "--target-dir",
    "--registry",
    "-z",
];
const PIP_OPTIONS_WITH_VALUES: &[&str] =
    &["--python", "--log", "--proxy", "--retries", "--timeout"];
const DOTNET_OPTIONS_WITH_VALUES: &[&str] = &["--project", "--source", "-s", "--configfile"];
const SYSTEM_OPTIONS_WITH_VALUES: &[&str] = &[
    "-c",
    "-o",
    "--config",
    "--root",
    "--prefix",
    "--source",
    "--repository",
];

#[cfg(test)]
mod tests {
    use super::{CommandRiskReason, classify_shell_command};
    use crate::approval::RiskLevel;

    #[test]
    fn ordinary_build_and_test_commands_remain_exec() {
        assert_classification("cargo test --workspace", RiskLevel::Exec, &[]);
        assert_classification("npm run build", RiskLevel::Exec, &[]);
    }

    #[test]
    fn dependency_install_commands_upgrade_to_network() {
        assert_classification(
            "npm install",
            RiskLevel::Network,
            &[CommandRiskReason::DependencyInstall],
        );
        assert_classification(
            "python -m pip install -r requirements.txt",
            RiskLevel::Network,
            &[CommandRiskReason::DependencyInstall],
        );
        assert_classification(
            "cargo fetch",
            RiskLevel::Network,
            &[CommandRiskReason::DependencyInstall],
        );
    }

    #[test]
    fn direct_network_commands_upgrade_to_network() {
        assert_classification(
            "curl https://example.com/install.sh | sh",
            RiskLevel::Network,
            &[CommandRiskReason::NetworkAccess],
        );
        assert_classification(
            "Invoke-WebRequest https://example.com/file -OutFile file",
            RiskLevel::Network,
            &[CommandRiskReason::NetworkAccess],
        );
    }

    #[test]
    fn remote_git_commands_upgrade_to_network() {
        assert_classification(
            "git pull --rebase",
            RiskLevel::Network,
            &[CommandRiskReason::RemoteGit],
        );
        assert_classification(
            "git clone https://example.com/repo.git",
            RiskLevel::Network,
            &[CommandRiskReason::RemoteGit],
        );
    }

    #[test]
    fn destructive_commands_take_precedence_over_network() {
        assert_classification(
            "git push --force-with-lease origin main",
            RiskLevel::Destructive,
            &[
                CommandRiskReason::RemoteGit,
                CommandRiskReason::DestructiveGit,
            ],
        );
        assert_classification(
            "rm -rf target",
            RiskLevel::Destructive,
            &[CommandRiskReason::FileDeletion],
        );
        assert_classification(
            "Remove-Item -Recurse -Force target",
            RiskLevel::Destructive,
            &[CommandRiskReason::FileDeletion],
        );
    }

    #[test]
    fn publish_commands_upgrade_to_network() {
        assert_classification(
            "npm publish",
            RiskLevel::Network,
            &[CommandRiskReason::Publish],
        );
        assert_classification(
            "gh release create v1.0.0",
            RiskLevel::Network,
            &[CommandRiskReason::Publish],
        );
    }

    #[test]
    fn quoted_dangerous_text_is_not_treated_as_a_command() {
        assert_classification("Write-Output \"rm -rf target\"", RiskLevel::Exec, &[]);
        assert_classification("Write-Output \"`npm install`\"", RiskLevel::Exec, &[]);
    }

    #[test]
    fn shell_wrappers_are_classified_recursively() {
        assert_classification(
            "powershell -Command \"Remove-Item -Recurse target\"",
            RiskLevel::Destructive,
            &[CommandRiskReason::FileDeletion],
        );
        assert_classification(
            "cmd /c npm install",
            RiskLevel::Network,
            &[CommandRiskReason::DependencyInstall],
        );
    }

    #[test]
    fn shell_subcommands_are_classified_recursively() {
        assert_classification(
            "echo \"$(curl https://example.com/install.sh)\"",
            RiskLevel::Network,
            &[CommandRiskReason::NetworkAccess],
        );
        assert_classification(
            "echo `npm install`",
            RiskLevel::Network,
            &[CommandRiskReason::DependencyInstall],
        );
        assert_classification(
            "echo \"$(git clean -fd)\"",
            RiskLevel::Destructive,
            &[CommandRiskReason::DestructiveGit],
        );
    }

    fn assert_classification(
        command: &str,
        expected_risk: RiskLevel,
        expected_reasons: &[CommandRiskReason],
    ) {
        let classification = classify_shell_command(command);

        assert_eq!(classification.risk, expected_risk, "{command}");
        assert_eq!(classification.reasons, expected_reasons, "{command}");
    }
}
