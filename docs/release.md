# 发布策略

状态：草案。

`deepseek-coder` 是 `AGPL-3.0-or-later` 自由软件。发布流程应清楚说明源码可获得性和构建可复现性。

## 发布产物

计划产物：

- 源码归档
- Rust CLI/TUI 二进制
- npm wrapper package
- VS Code `.vsix`
- 校验和
- release notes

## 要求

- 每个二进制或打包发布都要提供对应源码。
- 包含 `LICENSE`。
- 为网络服务部署提供源码获取说明。
- 提交 `Cargo.lock` 和 `pnpm-lock.yaml`。
- 稳定发布前提供可复现构建说明。

## 渠道

初始发布渠道可以包括：

- GitHub Releases
- 当 crate 准备好公开使用后发布到 Cargo
- 用于安装 wrapper 的 npm package
- VS Code Marketplace 或 Open VSX
