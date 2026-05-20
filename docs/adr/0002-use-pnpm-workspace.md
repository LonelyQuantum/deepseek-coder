# 架构决策记录 0002：使用 pnpm Workspace

状态：已接受。

## 背景

TypeScript 侧会包含 VS Code 插件、共享协议包，以及未来可能出现的 UI 或打包相关 package。

## 决策

使用 pnpm workspace，并提交 `pnpm-lock.yaml`。

## 影响

- 依赖解析比扁平化 npm 安装更严格。
- 共享 package 可以统一类型检查。
- `pnpm install --frozen-lockfile` 可以复现安装结果。
- 贡献者需要启用 Corepack 或安装 pnpm。
