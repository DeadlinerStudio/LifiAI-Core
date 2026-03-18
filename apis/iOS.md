# Deadliner AI Core - iOS Integration Guide

## 1. Recommended Repo Layout
Use this repository as an external engine workspace, not as code pasted into the iOS app target.

Recommended in another project directory:

```bash
git submodule add <your-github-url>/deadliner_core.git Vendor/deadliner_core
cd Vendor/deadliner_core
./scripts/build_ios_xc.sh
```

This produces:

- `dist/ios/ffi_uniffi.xcframework`
- `dist/ios/bindings/*.swift`

Then in Xcode:

1. Drag `ffi_uniffi.xcframework` into the app project.
2. Add the generated Swift files from `dist/ios/bindings/`.
3. Keep the Rust repo outside the app business code. Rebuild the framework only when the engine changes.

## 2. Runtime Ownership Model
Rust is the reasoning engine.
iOS is the source of truth for persistence.

Recommended ownership:

- Tasks / habits / memory persistence: Swift / CoreData / SQLite / Realm
- LLM orchestration / prompt / tool routing: Rust
- Memory storage in Rust: session mirror only

That means the app should hydrate memory into Rust on launch, and persist Rust deltas after each finished turn.

## 3. Build
From the repo root:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
./scripts/build_ios_xc.sh
```

## 4. Initialization
```swift
import Foundation

let storagePath = FileManager.default
    .urls(for: .documentDirectory, in: .userDomainMask)
    .first!
    .path

let core = DeadlinerCore(
    apiKey: "...",
    baseUrl: "https://api.deepseek.com/v1",
    modelId: "deepseek-chat",
    storagePath: storagePath,
    platform: "ios"
)
```

## 5. Memory Sync Protocol
### Startup hydration
At app launch, load the platform snapshot and send it into Rust:

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

### Delta write-back
When `OnFinish` arrives, inspect `memorySyncJson`.

If it is not `nil`, apply the operations to native storage transactionally, then native storage becomes the new source of truth.

Operation shapes:

- `UpsertFragment`
- `DeleteFragment`
- `ReplaceUserProfile`

Suggested native handling:

1. Read `baseRevision`.
2. Verify it matches your local memory revision.
3. Apply operations in one transaction.
4. Advance local revision to `nextRevision`.
5. If revisions diverge, ignore the delta and rehydrate Rust with a full snapshot.

### Full resync
If the app modifies memory outside Rust, call:

```swift
await core.replaceMemorySnapshot(snapshotJson: latestSnapshotJson)
```

You can also inspect the Rust-side mirror:

```swift
let snapshotJson = core.exportMemorySnapshot()
```

## 6. Callback Example
```swift
final class AICoordinator: CoreCallback {
    func onEvent(event: CoreEvent) {
        switch event {
        case .onThinking(let agentName):
            print("Thinking: \(agentName)")

        case .onToolRequest(let id, let toolName, let argsJson):
            handleTool(id: id, toolName: toolName, argsJson: argsJson)

        case .onFinish(let primaryIntent, let tasks, let habits, let chatResponse, let sessionSummary, let memorySyncJson):
            if let memorySyncJson {
                applyMemoryDelta(memorySyncJson)
            }
            print(primaryIntent)
            print(tasks ?? [])
            print(habits ?? [])
            print(chatResponse ?? "")
            print(sessionSummary ?? "")

        case .onError(let message):
            print("Error: \(message)")

        default:
            break
        }
    }
}
```

## 7. Pull-Based Fallback APIs
If you want a simpler first integration before fully wiring callback-driven state management, these APIs are available:

- `getLatestTasks()`
- `getLatestHabits()`
- `getLastMemorySyncJson()`
- `getLastFinishJson()`
- `getUserProfile()`
- `exportMemorySnapshot()`

This is useful during native debugging, snapshot testing, or when you want to verify Rust output before integrating full reducer/state logic.

## 8. Tool Feedback
When Swift finishes a local tool query:

```swift
Task {
    let resultJson = "{\"tasks\":[]}"
    await core.submitToolResult(id: toolCallId, resultJson: resultJson)
}
```
