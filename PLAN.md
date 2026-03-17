# Deadliner AI Core 跨平台重构技术方案 (V1.1)

## 1. 技术选型 (Tech Stack Selection)

- **核心语言**：**Rust**。利用其无 GC、内存安全、高性能的特性，承载大模型网络通信、Prompt 编排、Agent 调度及 JSON 解析。
- **跨平台 FFI 桥接**：
  - **iOS (Swift) / Android (Kotlin)**：使用 Mozilla 开源的 `uniffi-rs`。只需在 Rust 中定义接口（UDL 或 Macro），即可自动生成双端安全绑定的原生代码。
  - **HarmonyOS (ArkTS)**：使用 NAPI-RS (Node-API)。利用鸿蒙原生对 C/C++ NDK 的支持，直接桥接 ArkTS 和 Rust 核心。
- **核心架构**：**DDD (领域驱动设计) + Event-Driven (事件驱动)**。Rust 作为主动中枢（控制反转），原生层（SwiftUI / Compose / ArkUI）降级为被动的“UI 渲染器”与“本地数据提供者”。
- **JSON 库**：`serde` + `serde_json`，用于极速处理 LLM 数据映射与跨语言通信载荷。

## 2. 分层架构设计 (Layered Architecture Design)

基于 DDD 思想，Rust 核心库内部将严格分为四层，确保核心 AI 逻辑不被第三方网络库或平台特定的代码污染：

1. **Domain 层 (领域层)**：
   - **职责**：定义最核心的业务实体（如 `AITask`, `AIHabit`, `MixedResult`）和抽象接口（Traits）。
   - **内容**：包含 `ToolAgent`（子 Agent 的能力描述规范）和 `LlmGateway`（大模型抽象接口）。此层**绝对不依赖**任何外部网络框架或 FFI 库，是纯粹的业务模型。
2. **Application 层 (应用层)**：
   - **职责**：业务用例（Use Cases）与工作流编排。
   - **内容**：包含 `AgentOrchestrator`（调度器）。负责接收输入、组装 Prompt、调用 LLM 接口、解析 JSON、触发 Tool Call 拦截、以及合并 Native 回灌的 Observation 再次发起推理。
3. **Infrastructure 层 (基础设施层)**：
   - **职责**：为 Domain 层的抽象接口提供具体的技术实现。
   - **内容**：基于 `reqwest` 实现的 `DeepSeekClient`（处理真实的 HTTP 请求、Token 鉴权、重试机制），以及本地 Memory 的文件系统/KV 存储实现。
4. **Interface 层 (接口层 / FFI 边界)**：
   - **职责**：抹平语言边界，处理 Rust 与原生端（Swift/Kotlin/ArkTS）的类型转换和异步事件流。
   - **内容**：`uniffi` 或 `NAPI` 的入口函数，以及暴露给原生端的 `Event Callback` 状态机。

## 3. Rust-原生交互协议 (FFI Boundary)

由于 Agent 思考和执行是一个长耗时的异步过程，并且需要中途请求本地数据库（Repo），通信范式必须从“请求-响应”改为“事件流驱动”。

**接口契约 (Interface 层暴露给 Native)：**

- `init_core(config_json: String, platform: String) -> Arc<DeadlinerCore>`：初始化引擎，传入时区、平台标识（`ios`, `android`, `harmony`）等静态配置。
- `Messages(text: String)`：原生层发送用户输入。
- `submit_tool_observation(tool_call_id: String, result_json: String)`：**核心机制**。原生端执行完本地查询后，将 DTO 序列化为 JSON 字符串，回灌给 Rust 核心以继续推导。

**事件回调 (Native 监听 Core 的状态流)：**

原生端注册一个 Callback，接收以下枚举事件（Event Stream）：

- `OnThinking(agent_name)`：通知 UI 显示“思考中”或“已交由任务管家处理”。
- `OnToolRequest(id, tool_name, args_json)`：唤醒原生端去查询本地 Repo。
- `OnTextStream(chunk)`：打字机效果输出暖心回复。
- `OnFinish(mixed_result_json)`：流程结束，下发最终解析出的 Tasks/Habits，原生端执行落库并更新 UI。

## 4. 多 Agent 调度架构 (Multi-Agent Orchestration)

彻底废弃单体长 Prompt，在 Application 层采用 **Supervisor-Worker (总监-打工人)** 模式，按领域（Domain）隔离逻辑，极致节省 Token 并降低幻觉。

- **Supervisor Agent (路由中枢)**：
  - **职责**：意图分类、任务分发、合并结果。
  - **机制**：只看用户的原话，判断应该交给谁。例如发现既有任务又有习惯，则并行拉起 TaskAgent 和 HabitAgent。
- **Worker Agents (领域专家)**：
  - **TaskAgent**：只包含日程时间推算规则（如“明天早上默认 9 点”），只挂载 `read_tasks`、`update_deadline` 等任务相关的 Tools。
  - **HabitAgent**：只包含周期、频率的校验规则，挂载打卡相关的 Tools。
  - **ChatAgent**：处理闲聊和情绪安抚，无需挂载本地 Tools。

## 5. Tool 设计与平台差异隔离 (Tool Metadata Schema)

Tool (Skill) 的定义不再硬编码，而是作为 JSON 配置文件或 Domain 层的 Rust Struct，包含双重视角：既给 LLM 看，也给调度引擎看。

**Tool 定义数据结构标准：**

JSON

```
{
  "name": "read_tasks",
  "description": "读取用户本地的任务列表以进行状态分析或避免重复添加",
  "domain": "TaskAgent", 
  "supported_platforms": ["ios", "android", "harmony"], 
  "requires_ui_confirmation": false,
  "parameters": {
    "type": "object",
    "properties": {
      "timeRangeDays": { "type": "integer", "default": 7 },
      "keywords": { "type": "array", "items": { "type": "string" } }
    }
  }
}
```

**平台控制逻辑**：如果未来开发了仅支持 Apple Health 的 `read_sleep_data` 工具，将其 `supported_platforms` 设为 `["ios"]`。当引擎在安卓或鸿蒙端初始化时，Application 层的调度器会自动将其从上下文剔除。

## 6. Prompt 动态拼接策略 (Prompt Assembly)

Prompt 组装工作下沉至 Application 层，采用模块化动态拼接，彻底告别超大硬编码字符串：

1. **System Base**：全局时间、时区、用户当前偏好（Memory 注入）。
2. **Platform Tool Filter**：根据初始化时传入的 `platform` 标识，遍历该 Worker Agent 所属的 Tools，仅拼接当前平台支持的 Tool Schema。
3. **Observation Injection**：当原生端调用 `submit_tool_observation` 回灌数据时，触发二段 Prompt 拼接：将原始 Input + `[Tool Result JSON]` 再次提交给 LLM，要求给出最终决策。

## 7. 原生端 Tool 注册与分发 (Native Tool Execution)

虽然业务中枢在 Rust，但核心的持久化落盘在原生端。原生端需要建立一个本地 Tool 分发器。

**执行流：**

1. 原生端的 `Event Callback` 收到 `OnToolRequest(id = "123", tool_name = "read_tasks", args_json = "...")`。
2. 原生层的 **Tool Dispatcher** 拦截该请求。
3. 反序列化 `args_json`，调用本地 Repo（例如通过 ArkTS 查 RelationalStore，通过 Swift 查 CoreData，通过 Kotlin 查 Room）。
4. 查询得到原生 DTO（三端已统一）。
5. 将 DTO 序列化为标准的 JSON 字符串 `result_json`。
6. 原生层调用 `core.submit_tool_observation("123", result_json)` 将数据送回黑盒。