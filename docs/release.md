# 发布策略

状态：草案。

`ProleCoder` 是 `AGPL-3.0-or-later` 自由软件。发布流程应清楚说明源码可获得性和构建可复现性。

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

## 后续增强

- 添加 `LICENSE` 文件，并在发布包中包含 AGPL-3.0-or-later 许可证文本和源码获取说明。
- 设计可复现构建流程，记录 Rust、Node.js、pnpm、VSIX 打包工具和平台目标版本。
- 增加发布前检查：格式、lint、测试、敏感信息扫描、依赖审计、产物校验和变更日志生成。
- 为 CLI/TUI 二进制、npm wrapper 和 VSIX 生成校验和，并在 GitHub Release 中发布。
- 明确网络服务部署场景下的源码提供方式，避免 AGPL 合规说明留到发布后补救。
