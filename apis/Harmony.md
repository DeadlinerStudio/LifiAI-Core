# Deadliner AI Core 鸿蒙接入文档

## 1. 当前状态

鸿蒙目前采用单独的 `ffi_ohos` 边界层，不与 iOS / Android 共用 UniFFI 胶水层。

当前 workspace 结构：

- `core_logic`：纯引擎逻辑
- `ffi_uniffi`：iOS / Android 接口层
- `ffi_ohos`：鸿蒙接口层

这个拆分是刻意保留的，目的是避免把 UniFFI 宏和鸿蒙专用导出方式混在同一个 crate 里。

## 2. 你现在做鸿蒙时应该怎么理解

基于当前仓库状态，鸿蒙接入分成两层：

1. `core_logic` 和 `DeadlinerCore` 已经可复用。
2. `ffi_ohos` 现在还只是“鸿蒙专用壳层”，尚未补齐真正的 OHOS 导出代码。

也就是说：

- 引擎逻辑已经有了
- 鸿蒙 FFI 边界还没最终落地
- 你接下来要做的重点，是在 `ffi_ohos` 中补 ArkTS/NAPI/ohrs 所需导出

## 3. 当前推荐接入策略

建议仍然把本仓库作为独立引擎仓库挂到鸿蒙工程外层：

```bash
git submodule add <your-github-url>/deadliner_core.git vendor/deadliner_core
cd vendor/deadliner_core
./scripts/build_ohos.sh
```

当前 `build_ohos.sh` 的行为是：

- 如果本机安装了 `ohrs`，则执行 `ohrs build -p ffi_ohos`
- 如果没有安装 `ohrs`，则只执行 `cargo build --release -p ffi_ohos`
- 如果安装了 `ohrs` 但没有设置 `OHOS_NDK_HOME`，脚本会直接失败并提示先补齐 NDK 环境
- 现在也会自动尝试从 `HOS_SDK_HOME/native` 推导 `OHOS_NDK_HOME`

因此它现在更像：

- 已安装鸿蒙工具链时：用于生成真实鸿蒙产物
- 未安装工具链时：用于做 Rust 结构层面的构建校验

在已安装 `ohrs` 的机器上，至少应先满足：

- `OHOS_NDK_HOME` 已指向 OpenHarmony NDK
- `rustup target add aarch64-unknown-linux-ohos`
- 推荐额外安装 `armv7-unknown-linux-ohos` 与 `x86_64-unknown-linux-ohos`，因为 `ohrs doctor` 会检查它们

另外，`core_logic` 已切换为 `reqwest + rustls-tls`，这样 OHOS 构建不再依赖目标平台 OpenSSL 交叉编译环境。

## 3.1 CI 推荐分层

当前仓库的 GitHub Actions 现在只保留一条真正的鸿蒙产物线：

- `HarmonyOS Build`：安装 `ohrs`、安装 Harmony SDK、导出 `OHOS_NDK_HOME`，然后执行 [`scripts/build_ohos.sh`](/Users/aritxonly/Codes/Agent/deadliner_core/scripts/build_ohos.sh)

这条 CI 的原则是：

- 只接受真实 `ohrs build`
- 如果没有 `ohrs`，直接失败
- 如果没有 Harmony SDK / `OHOS_NDK_HOME`，直接失败
- 如果缺少 OpenHarmony Rust targets，直接失败
- 不再回退到 `cargo build` 生成 host validation artifact

CI 侧当前可配置的 GitHub Actions Variables：

- `HARMONY_SDK_RELEASE_REPO`：可选，默认 `harmonyos-dev/hos-sdk`
- `HARMONY_SDK_FULL_DOWNLOAD_URL`：可选，优先使用的完整 Harmony SDK 镜像下载地址
- `HARMONY_SDK_FULL_ASSET_PATTERN`：可选，用于从 release 中匹配完整 SDK 镜像
- `HARMONY_SDK_ASSET_PATTERN`：可选，用于匹配 command-line tools 压缩包
- `HARMONY_SDK_DOWNLOAD_URL`：可选，直接指定 command-line tools 下载地址
- `HARMONY_SDK_PACKAGES`：可选，默认 `toolchains:9 OpenHarmony/toolchains:9`

如果 SDK release 是私有的，还可以额外配置：

- `HARMONY_SDK_GITHUB_TOKEN`：用于下载私有 GitHub Release 资源

对应脚本入口：

- [`scripts/setup_harmony_sdk.sh`](/Users/aritxonly/Codes/Agent/deadliner_core/scripts/setup_harmony_sdk.sh)

推荐优先策略：

- 如果 release 里已经有完整 `hwsdk` / `sdk` 镜像，就优先配置 `HARMONY_SDK_FULL_DOWNLOAD_URL` 或 `HARMONY_SDK_FULL_ASSET_PATTERN`
- 只有没有完整 SDK 镜像时，才退回 `command-line-tools + sdkmgr install`
- 当前 workflow 默认已提供一个公开完整 SDK 地址：`https://repo.huaweicloud.com/openharmony/os/5.1.0-Release/L2-SDK-MAC-M1-PUBLIC.tar.gz`

原因是后者仍然依赖远端服务在线返回 toolchains 下载地址，在 GitHub Actions 上稳定性较差。

## 4. 职责划分

建议鸿蒙侧与 iOS / Android 保持一致：

- Rust 负责：LLM 调用、意图识别、工具编排、记忆推理
- 鸿蒙侧负责：任务/习惯/记忆持久化、页面状态、系统能力和设备能力
- Rust 中记忆：只保留活跃会话期间的同步镜像

鸿蒙侧本地存储可以使用：

- `RelationalStore`
- `Preferences`
- 或你自己的持久化封装

## 5. 当前可复用的核心能力

`ffi_ohos` 当前直接 re-export 了以下 Rust 类型：

- `DeadlinerCore`
- `CoreCallback`
- `CoreEvent`
- `FFITask`
- `FFIHabit`
- `CoreError`
- `extract_tasks(...)`

`DeadlinerCore` 当前可用的核心能力与其他平台一致：

- `set_callback(...)`
- `process_input(...)`
- `submit_tool_result(...)`
- `get_latest_tasks()`
- `get_latest_habits()`
- `get_last_memory_sync_json()`
- `get_last_finish_json()`
- `get_user_profile()`
- `replace_memory_snapshot(...)`
- `export_memory_snapshot()`

## 6. 事件模型

鸿蒙侧真正需要桥接出去的事件如下：

- `OnLifecycle { request_id, stage, status, message }`
- `OnThinking { agent_name, phase, message }`
- `OnTextStream { chunk }`
- `OnToolRequest { id, tool_name, args_json }`
- `OnFinish { primary_intent, tasks, habits, chat_response, session_summary, memory_sync_json }`
- `OnMemoryCommitted { added_memories, profile_updated, new_revision }`
- `OnError { message }`

当前实现里有一个很关键的实际行为：

1. `OnFinish.memory_sync_json` 目前固定为空。
2. 记忆真正提交后，会额外触发 `OnMemoryCommitted`。
3. 最近一次记忆增量需要通过 `get_last_memory_sync_json()` 获取。

所以鸿蒙侧桥接时，推荐把“结果完成”和“记忆提交”分成两个事件来处理。

推荐统一按 `OnLifecycle` 驱动宿主状态机，阶段定义见 [Lifecycle.md](/Users/aritxonly/Codes/Agent/deadliner_core/apis/Lifecycle.md)。

## 7. 记忆同步协议

### 启动时全量注水

当鸿蒙应用启动，或者本地数据比 Rust 更新时，先把本地权威快照写入 Rust：

```json
{
  "revision": 12,
  "fragments": [
    {
      "id": "7e0e0a5d-9a49-4e0d-8a16-5d2d89a7d41f",
      "content": "用户偏好晨间处理高优先级事项",
      "category": "assistant_inferred",
      "timestamp": "2026-03-18T09:30:00Z",
      "importance": 3
    }
  ],
  "userProfile": "偏好中文回复"
}
```

对应调用思路：

- ArkTS/Native 层拿到快照 JSON
- 调 `replace_memory_snapshot(snapshot_json)`

### 完成后的增量回写

推荐流程：

1. 收到 `OnFinish` 后更新 UI 和任务/习惯展示。
2. 收到 `OnMemoryCommitted` 后，调用 `get_last_memory_sync_json()` 读取最近一次记忆增量。
3. 若增量不为空，则在本地存储事务中应用。

建议事务规则：

1. 校验 `baseRevision` 与本地 revision 一致
2. 在一个事务中应用所有操作
3. 把本地 revision 推进到 `nextRevision`
4. 如果版本冲突，则丢弃该增量并重新做一次全量 `replace_memory_snapshot(...)`

当前文档口径支持的操作：

- `UpsertFragment`
- `DeleteFragment`
- `ReplaceUserProfile`

### 全量导出

调试或兜底恢复时，可以直接导出 Rust 当前镜像：

```text
core.export_memory_snapshot()
```

## 8. 工具调用链路

当 Rust 发出 `OnToolRequest` 后，鸿蒙侧需要：

1. 解析 `tool_name` 与 `args_json`
2. 查询本地仓库、数据库或系统能力
3. 序列化结果 JSON
4. 回调 `submit_tool_result(id, result_json)`

当 Rust 发出 `OnFinish` 后，鸿蒙侧需要：

1. 刷新任务、习惯、聊天结果
2. 等待后续可能到来的 `OnMemoryCommitted`

## 9. 当前最重要的实现落点

你马上做鸿蒙时，最应该落代码的位置是：

[ffi_ohos/src/lib.rs](/Users/aritxonly/Codes/Agent/deadliner_core/ffi_ohos/src/lib.rs)

当前这个文件已经补上了一版最小可用的 OHOS/NAPI 导出层：

- 暴露了 `DeadlinerCore` JS 类
- 暴露了 `process_input` / `submit_tool_result` / 记忆同步相关方法
- 暴露了 `get_latest_tasks_json` / `get_latest_habits_json`
- 把 `CoreEvent` 统一转成 JSON 字符串并进入 Rust 侧事件队列
- 鸿蒙侧当前通过 `drain_events_json()` 轮询消费事件，而不是实时 callback

因此下一步建议集中做这几件事：

1. 在 ArkTS 里先把 `DeadlinerCore` 的主链路接通
2. 轮询 `drain_events_json()` 并分发 `thinking` / `toolRequest` / `finish` / `memoryCommitted` / `error`
3. 把 JSON 事件再转成你项目里的类型
4. 如果后续需要实时 callback，再补线程安全的 JS callback 桥

## 10. 建议的最小可用接入顺序

为了尽快开始鸿蒙开发，建议按下面顺序落地：

1. 先在 `ffi_ohos` 暴露 `DeadlinerCore.new`
2. 暴露 `process_input`
3. 轮询 `drain_events_json()`
4. 在 `toolRequest` 事件里执行本地工具，再调用 `submit_tool_result`
5. 在 `finish` 事件后读取 `get_latest_tasks_json` / `get_latest_habits_json`
6. 最后接 `replace_memory_snapshot`、`get_last_memory_sync_json`、`export_memory_snapshot`

这样你可以先把“输入一句话 -> Rust 推理 -> 鸿蒙轮询事件 -> 鸿蒙执行工具 -> 回传结果 -> 页面展示”这条主链路跑通，再补更实时的事件桥接。
