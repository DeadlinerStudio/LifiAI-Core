# Deadliner AI Core Android 接入文档

## 1. 当前接入方式

建议把本仓库作为独立引擎仓库接入 Android 项目。

推荐在 Android 项目外层目录中执行：

```bash
git submodule add <your-github-url>/deadliner_core.git vendor/deadliner_core
cd vendor/deadliner_core
./scripts/build_android.sh
```

构建产物：

- `dist/android/jniLibs/.../*.so`
- `dist/android/bindings/*.kt`

当前默认 Kotlin 包名：

- `com.aritxonly.deadliner.lifi`

接入时：

1. 把 `jniLibs` 放进 Android module。
2. 把生成的 Kotlin bindings 纳入源码目录。
3. 把 Rust 引擎版本升级和业务代码解耦。

## 2. 职责划分

建议的边界如下：

- Rust 负责：意图识别、任务/习惯解析、工具编排、记忆推理
- Android 负责：Room/SQLDelight/Realm 等本地持久化、UI 状态、系统能力
- Rust 中记忆：只保留运行期同步镜像

## 3. 构建

```bash
cargo install cargo-ndk
./scripts/build_android.sh
```

如需显式覆盖构建参数：

```bash
CRATE_NAME=ffi_uniffi \
LIB_NAME=ffi_uniffi \
KOTLIN_PACKAGE=com.aritxonly.deadliner.lifi \
./scripts/build_android.sh
```

## 4. 初始化

```kotlin
val storagePath = context.filesDir.absolutePath

val core = DeadlinerCore(
    apiKey = "...",
    baseUrl = "https://api.deepseek.com/v1",
    modelId = "deepseek-chat",
    storagePath = storagePath,
    platform = "android"
)
```

当前仓库默认配置口径：

- `DEEPSEEK_API_KEY`
- `DEEPSEEK_BASE_URL`
- `DEEPSEEK_MODEL`

## 5. 对外核心 API

当前可直接使用的核心接口：

- `setCallback(...)`
- `processInput(...)`
- `submitToolResult(...)`
- `getLatestTasks()`
- `getLatestHabits()`
- `getLastMemorySyncJson()`
- `getLastFinishJson()`
- `getUserProfile()`
- `replaceMemorySnapshot(...)`
- `exportMemorySnapshot()`

## 6. 事件模型

当前实际事件与 iOS 保持一致：

- `OnThinking`
- `OnTextStream`
- `OnToolRequest`
- `OnFinish`
- `OnMemoryCommitted`
- `OnError`

需要注意的实现细节：

1. `OnFinish.memorySyncJson` 当前实现里是空值。
2. 记忆提交结果通过 `OnMemoryCommitted` 单独通知。
3. 记忆增量 JSON 需要通过 `getLastMemorySyncJson()` 主动读取。

## 7. 记忆同步协议

### 启动时注水

Android 启动时先把本地权威快照写入 Rust：

```kotlin
val snapshotJson = """
{
  "revision": 12,
  "fragments": [],
  "userProfile": "偏好中文回复"
}
""".trimIndent()

scope.launch {
    core.replaceMemorySnapshot(snapshotJson)
}
```

### 增量回写

推荐处理顺序：

1. 收到 `OnFinish` 后先更新页面状态。
2. 收到 `OnMemoryCommitted` 后调用 `getLastMemorySyncJson()`。
3. 若返回非空，则在本地数据库事务中应用增量。

建议的事务逻辑：

1. 校验 `baseRevision` 是否等于 Android 本地 revision
2. 在一个事务中应用全部操作
3. 成功后把 revision 推进到 `nextRevision`
4. 一旦冲突，直接重建全量快照并重新执行 `replaceMemorySnapshot(...)`

支持的操作类型：

- `UpsertFragment`
- `DeleteFragment`
- `ReplaceUserProfile`

### 全量导出

```kotlin
val snapshotJson = core.exportMemorySnapshot()
```

## 8. 回调示例

```kotlin
core.setCallback(object : CoreCallback {
    override fun onEvent(event: CoreEvent) {
        when (event) {
            is CoreEvent.OnThinking -> {
                // 展示推理中状态
            }

            is CoreEvent.OnTextStream -> {
                // 处理流式文本
            }

            is CoreEvent.OnToolRequest -> {
                handleTool(event.id, event.toolName, event.argsJson)
            }

            is CoreEvent.OnFinish -> {
                val tasks = event.tasks
                val habits = event.habits
                val chat = event.chatResponse
            }

            is CoreEvent.OnMemoryCommitted -> {
                core.getLastMemorySyncJson()?.let { applyMemoryDelta(it) }
            }

            is CoreEvent.OnError -> {
                // 展示错误
            }
        }
    }
})
```

## 9. 工具结果回传

当 Android 完成本地查询后：

```kotlin
scope.launch {
    val resultJson = """{"tasks":[]}"""
    core.submitToolResult(toolCallId, resultJson)
}
```

## 10. 渐进式接入建议

推荐先跑通最小链路，再逐步补完：

1. 先接 `processInput`、`OnToolRequest`、`submitToolResult`
2. 再用 `getLatestTasks()` / `getLatestHabits()` 校验结果
3. 接入 `OnFinish` 更新 UI
4. 最后补 `replaceMemorySnapshot()`、`OnMemoryCommitted`、`getLastMemorySyncJson()`
