# Deadliner AI Core - HarmonyOS (ArkTS) Integration Guide

## 1. Strategy
For HarmonyOS, we leverage **NAPI-RS** or a C-ABI bridge since `uniffi` doesn't natively support ArkTS yet.

## 2. Usage in ArkUI

### NAPI Bridge (Example)
```typescript
import deadliner from 'libdeadliner_core.so';

// Initialization
deadliner.initCore({
    apiKey: "...",
    storagePath: context.filesDir
});

// Subscription
deadliner.subscribe((event) => {
    if (event.type === 'OnFinish') {
        this.tasks = event.tasks;
    }
});

// Call
deadliner.processInput("帮我记一下明天的会议");
```

## 3. Tool Feedback
When ArkTS receives `OnToolRequest`, it queries the `RelationalStore` and feeds data back:
```typescript
let result = await queryDatabase(args);
deadliner.submitToolResult(id, JSON.stringify(result));
```
