# Security Policy

`deepseek-coder` is intended to read, edit, and run commands in developer workspaces. Security issues are taken seriously because the tool may handle source code, secrets, shell commands, model outputs, and local project state.

## Supported Versions

The project is pre-release. Security fixes target the default branch until versioned releases begin.

## Reporting a Vulnerability

Please do not open a public issue for a vulnerability before maintainers have had time to investigate.

Preferred reporting paths:

- Use a private GitHub security advisory if available.
- If private advisories are not available, contact a maintainer directly and include `SECURITY` in the subject.

Include:

- Affected commit, version, or branch.
- Reproduction steps.
- Expected and actual behavior.
- Impact assessment.
- Whether the issue can expose source code, secrets, shell access, network access, or model prompts.

## Security Scope

Issues in scope include:

- Secret leakage through logs, prompts, tool output, or telemetry.
- Unsafe command execution.
- Workspace path traversal.
- Unapproved file writes or destructive operations.
- Prompt or tool-call handling that bypasses approval policy.
- VS Code extension behavior that violates Workspace Trust expectations.
- Supply-chain risks in release, package, or CI configuration.

Issues out of scope include:

- Social engineering against maintainers.
- Vulnerabilities requiring a malicious local administrator.
- Denial of service through intentionally huge local files unless it bypasses documented limits.

## Handling Expectations

Maintainers should acknowledge valid reports, avoid public disclosure until a fix is ready, and credit reporters who want credit. If a report is not accepted, maintainers should explain why it is out of scope or not reproducible.

## Secrets

Never include real API keys, access tokens, private source code, or confidential prompts in public issues, pull requests, logs, screenshots, or test fixtures.
