# Deadliner AI Core - iOS Integration Guide

## 1. Build and Link
The Rust core is compiled into a static library (`libdeadliner_core.a`) and a Swift header via `uniffi`.

```bash
# Generate XCFramework (Simplified example)
cargo apple-build
```

## 2. Usage in Swift

### Initialization
```swift
import DeadlinerCore

let storagePath = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first!.path
let core = DeadlinerCore(
    apiKey: "your-key",
    baseUrl: "https://api.deepseek.com/v1",
    modelId: "deepseek-chat",
    storagePath: storagePath
)

// Register callback
core.setCallback(callback: self)
```

### Implementing Callback
```swift
class AICoordinator: CoreCallback {
    func onEvent(event: CoreEvent) {
        switch event {
        case .onThinking(let agent):
            print("AI is thinking: \(agent)")
        case .onToolRequest(let id, let name, let argsJson):
            handleLocalTool(id: id, name: name, args: argsJson)
        case .onFinish(let intent, let tasks, let habits, let chat, let summary):
            updateUI(tasks: tasks, chat: chat)
        case .onError(let msg):
            showError(msg)
        }
    }
}
```

### Sending Message
```swift
// Async call, returns immediately
core.processInput(text: "Remind me to buy milk tomorrow 10am")
```
