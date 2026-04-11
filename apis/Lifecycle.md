# Deadliner Core 统一生命周期规约

该规约适用于 iOS / Android / Harmony 三端，目的是让宿主层只依赖稳定生命周期协议，而不依赖内部 Agent 实现细节。

## 1. 生命周期事件

Core 通过 `OnLifecycle` 事件对外广播阶段状态：

- `requestId`: 一次 `processInput` 或 `submitToolResult` 调用的关联 ID
- `stage`: 当前阶段
- `status`: 阶段状态
- `message`: 可选补充信息

## 2. 阶段定义

- `process_input`
  - `started`: 收到用户输入，开始编排
  - `failed`: 本轮处理失败

- `submit_tool_result`
  - `started`: 收到工具回灌
  - `failed`: 回灌处理失败

- `tool_request`
  - `waiting`: Core 已发出 `OnToolRequest`，等待宿主执行工具

- `orchestration`
  - `completed`: 本轮主流程完成，已发出 `OnFinish`

- `memory_commit`
  - `completed`: 增量记忆已提交，并已发出 `OnMemoryCommitted`
  - `failed`: 记忆提交失败

## 3. 宿主端状态机建议

1. 收到 `process_input.started` 或 `submit_tool_result.started`：进入 `Running`
2. 收到 `tool_request.waiting`：进入 `WaitingTool`
3. 收到 `orchestration.completed`：进入 `Finished`
4. 收到 `memory_commit.completed`：可标记 `MemorySynced`
5. 收到任意 `*.failed`：进入 `Failed`

## 4. 兼容性原则

- 宿主端应按 `stage + status` 解析，不依赖内部 Agent 名称。
- 后续新增阶段时，未知 `stage` 应按“可忽略但可记录”处理，保证前向兼容。
