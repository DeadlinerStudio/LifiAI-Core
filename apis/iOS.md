# Deadliner AI Core iOS 接入文档

## 1. 当前接入方式

建议把本仓库作为独立引擎仓库接入，而不是把 Rust 代码直接拷进 iOS 工程。

推荐在你的 iOS 项目外层目录中执行：

```bash
git submodule add <your-github-url>/deadliner_core.git Vendor/deadliner_core
cd Vendor/deadliner_core
./scripts/build_ios_xc.sh
```

构建产物：

- `dist/ios/ffi_uniffi.xcframework`
- `dist/ios/bindings/*.swift`

在 Xcode 中：

1. 把 `ffi_uniffi.xcframework` 拖入工程。
2. 把 `dist/ios/bindings/` 下生成的 Swift 文件加入 app target。
3. 引擎升级时只重新构建产物，不把业务代码和 Rust 引擎混在一起。

## 2. 职责划分

当前建议的职责边界如下：

- Rust 负责：LLM 调用、意图解析、工具编排、记忆推理
- iOS 负责：任务/习惯/记忆的持久化、UI 状态管理、系统能力调用
- Rust 中的记忆存储：仅作为会话期间的同步镜像

也就是说，iOS 仍然是持久化数据的最终真相源，Rust 只是运行期推理引擎。

## 3. 构建

首次构建前需要准备 iOS target：

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
./scripts/build_ios_xc.sh
```

## 4. 初始化

当前构造函数签名如下：

```swift
let core = DeadlinerCore(
    apiKey: "...",
    baseUrl: "https://api.deepseek.com/v1",
    modelId: "deepseek-chat",
    storagePath: storagePath,
    platform: "ios"
)
```

建议 `storagePath` 使用应用沙盒内可写目录，例如：

```swift
import Foundation

let storagePath = FileManager.default
    .urls(for: .documentDirectory, in: .userDomainMask)
    .first!
    .path
```

目前仓库默认使用 DeepSeek，对应环境变量口径为：

- `DEEPSEEK_API_KEY`
- `DEEPSEEK_BASE_URL`
- `DEEPSEEK_MODEL`

## 5. 对外核心 API

当前 UniFFI 导出的主要能力如下：

- `setCallback(_:)`
- `processInput(text:)`
- `submitToolResult(id:resultJson:)`
- `getLatestTasks()`
- `getLatestHabits()`
- `getLastMemorySyncJson()`
- `getLastFinishJson()`
- `getUserProfile()`
- `replaceMemorySnapshot(snapshotJson:)`
- `exportMemorySnapshot()`

## 6. 事件模型

当前实际事件为：

- `OnThinking { agentName, phase, message }`
- `OnTextStream { chunk }`
- `OnToolRequest { id, toolName, argsJson }`
- `OnFinish { primaryIntent, tasks, habits, chatResponse, sessionSummary, memorySyncJson }`
- `OnMemoryCommitted { addedMemories, profileUpdated, newRevision }`
- `OnError { message }`

需要特别注意：

1. `OnFinish.memorySyncJson` 当前实现里固定为 `nil`。
2. 记忆提交完成后，会通过单独的 `OnMemoryCommitted` 事件通知。
3. 真正的记忆增量 JSON 需要通过 `getLastMemorySyncJson()` 拉取。

也就是说，iOS 侧不要再假设“`OnFinish` 一到就一定携带记忆增量”。

## 7. 记忆同步协议

### 启动时全量注水

应用启动时，先从 iOS 本地数据库读取权威快照，再灌入 Rust：

```swift
let snapshotJson = """
{
  "revision": 12,
  "fragments": [
    {
      "id": "7E0E0A5D-9A49-4E0D-8A16-5D2D89A7D41F",
      "content": "用户喜欢晨间处理高优先级任务",
      "category": "assistant_inferred",
      "timestamp": "2026-03-18T09:30:00Z",
      "importance": 3
    }
  ],
  "userProfile": "偏好中文回复，工作日早上安排任务"
}
"""

Task {
    await core.replaceMemorySnapshot(snapshotJson: snapshotJson)
}
```

### 完成后增量回写

当前推荐的处理方式是：

1. 收到 `OnFinish` 后更新任务、习惯和聊天输出。
2. 再监听 `OnMemoryCommitted`。
3. 收到 `OnMemoryCommitted` 后，调用 `getLastMemorySyncJson()` 取回最近一次记忆增量。
4. 如果增量不为空，在本地数据库中事务性应用该增量。

增量应用建议流程：

1. 读取本地 `revision`
2. 校验它是否等于 `baseRevision`
3. 在一个事务中应用全部操作
4. 把本地版本号推进到 `nextRevision`
5. 如果版本不一致，放弃本次增量并重新执行一次全量 `replaceMemorySnapshot(...)`

当前文档口径支持的操作类型：

- `UpsertFragment`
- `DeleteFragment`
- `ReplaceUserProfile`

### 调试导出

如果你要对比 Rust 内部镜像状态，可以直接拉全量快照：

```swift
let snapshotJson = core.exportMemorySnapshot()
```

## 8. 回调示例

```swift
final class AICoordinator: CoreCallback {
    func onEvent(event: CoreEvent) {
        switch event {
        case .onThinking(let agentName, let phase, let message):
            print("thinking", agentName, phase, message ?? "")

        case .onTextStream(let chunk):
            print("stream", chunk)

        case .onToolRequest(let id, let toolName, let argsJson):
            handleTool(id: id, toolName: toolName, argsJson: argsJson)

        case .onFinish(let primaryIntent, let tasks, let habits, let chatResponse, let sessionSummary, _):
            print(primaryIntent)
            print(tasks ?? [])
            print(habits ?? [])
            print(chatResponse ?? "")
            print(sessionSummary ?? "")

        case .onMemoryCommitted(_, _, _):
            if let memorySyncJson = core.getLastMemorySyncJson() {
                applyMemoryDelta(memorySyncJson)
            }

        case .onError(let message):
            print("error", message)

        default:
            break
        }
    }
}
```

## 9. 工具回调

当 Rust 发出 `OnToolRequest` 后，iOS 侧应：

1. 解析 `toolName` 与 `argsJson`
2. 查询本地仓库或系统能力
3. 把结果序列化成 JSON
4. 回调给 Rust

示例：

```swift
Task {
    let resultJson = "{\"tasks\":[]}"
    await core.submitToolResult(id: toolCallId, resultJson: resultJson)
}
```

## 10. 渐进式接入建议

如果你想先把链路跑通，再逐步做完整状态管理，建议顺序如下：

1. 先接 `processInput` + `OnToolRequest` + `submitToolResult`
2. 用 `getLatestTasks()` / `getLatestHabits()` 验证引擎输出
3. 再接 `OnFinish` 更新 UI
4. 最后接入 `replaceMemorySnapshot()`、`OnMemoryCommitted` 和 `getLastMemorySyncJson()`
