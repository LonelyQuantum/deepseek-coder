# 安全模型

状态：草案。

`deepseek-coder` 可以读取代码、写入文件、执行命令、调用模型提供商并展示生成的 patch。安全模型把模型输出和工作区内容都视为不可信输入。

## 边界

- API Key 不得进入 run log。
- `.env` 和本地状态必须被 git 忽略。
- tool call 执行前必须校验 schema。
- 写入应通过 patch application。
- 破坏性操作必须显式审批。
- 网络访问必须显式审批。

## 威胁

- 源码中的 prompt injection。
- 密钥通过日志或 prompt 泄漏。
- tool-call 参数伪造。
- 路径穿越到 workspace 外部。
- 未审阅的命令执行。
- 发布或 CI 依赖中的供应链风险。

## 初始缓解措施

- 显式审批模型。
- 结构化工具 schema。
- 带基础脱敏策略的本地 run log。
- 写入前检查 workspace 路径。
- CI 检查格式、lint、测试和类型。

## 后续增强

- 扩展统一脱敏层，覆盖更多 API Key 形态、环境变量、shell 输出、搜索结果、diff、run log 和 provider 错误正文。
- 为敏感路径建立可配置拒绝规则，默认覆盖 `.env`、`.secrets/`、`.secret/`、`.git/`、`.agents/`、证书、token 文件和常见云服务凭据。
- 增加命令风险分类器，在执行前识别网络访问、依赖安装、发布、远程 git 操作、删除和 reset 等高风险行为。
- 按平台实现并测试 sandbox 边界；Windows、Linux 和 macOS 的能力差异需要在文档和测试中分别说明。
- 在发布前增加敏感信息扫描、依赖审计和产物校验，确保本地路径、API Key 和临时文件不会进入源码包或 VSIX。
