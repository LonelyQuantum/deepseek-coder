# 贡献指南

感谢你参与 `ProleCoder`。这是一个使用 `AGPL-3.0-or-later` 许可证发布的自由软件代码 Agent，主要技术栈是 Rust 和 TypeScript。

## 开发环境

请先按照 `README.md` 中的开发环境配置完成本机准备。Windows 下建议在新的 PowerShell 窗口中确认工具链：

```powershell
git --version
rustc --version
cargo --version
node --version
pnpm --version
rg --version
```

在仓库根目录安装项目依赖：

```powershell
pnpm install
```

## 提交前检查

提交前运行和 CI 一致的检查：

```powershell
pnpm run check
```

局部开发时也可以单独运行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -r typecheck
```

## Pull Request 要求

- 每个 PR 聚焦一个主题。
- 说明用户可见行为、工程取舍和验证方式。
- 修改行为时补充测试。
- 完成开发计划中的事项时，同步更新 `README.md` 的 checkbox。
- 不提交本地密钥、`.env`、`node_modules`、`target`、构建产物或 run log。
- 依赖变化时提交锁文件：`Cargo.lock` 和 `pnpm-lock.yaml`。

## 代码风格

- Rust 使用 `rustfmt`、Clippy，并禁止 `unsafe_code`。
- TypeScript 使用严格编译选项和共享 workspace 配置。
- 优先显式错误，不用静默兜底掩盖问题。
- 协议类型应集中在共享包中，避免在不同前端复制临时结构。

## 许可证

提交贡献即表示你同意该贡献按 `AGPL-3.0-or-later` 授权。
