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
- 带脱敏策略的本地 run log。
- 写入前检查 workspace 路径。
- CI 检查格式、lint、测试和类型。
