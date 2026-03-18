# Deadliner AI Core - Android Integration Guide

## 1. Recommended Consumption Pattern
Treat this repository as a separately versioned engine workspace.

Recommended in another Android project:

```bash
git submodule add <your-github-url>/deadliner_core.git vendor/deadliner_core
cd vendor/deadliner_core
./scripts/build_android.sh
```

This produces:

- `dist/android/jniLibs/.../*.so`
- `dist/android/bindings/*.kt`

Then in Android Studio:

1. Copy `jniLibs` into the Android module.
2. Add the generated Kotlin bindings into a source set.
3. Keep Rust engine updates isolated from app business code.

## 2. Ownership Model
Android owns persistence.
Rust owns reasoning and orchestration.

Recommended native stores:

- Room / SQLDelight / Realm for tasks, habits, memory
- Rust only keeps a synchronized in-memory mirror for prompt context and write-back deltas

## 3. Build
```bash
cargo install cargo-ndk
./scripts/build_android.sh
```

## 4. Initialization
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

## 5. Memory Sync Protocol
### Startup hydration
Load the Android-side canonical snapshot and hydrate Rust:

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

### Delta write-back
When Rust emits `OnFinish`, check `memorySyncJson`.

If present:

1. Verify `baseRevision` matches the local Android memory revision.
2. Apply every operation in one DB transaction.
3. Advance local revision to `nextRevision`.
4. If mismatch occurs, rebuild a full snapshot and call `replaceMemorySnapshot(...)`.

Supported operations:

- `UpsertFragment`
- `DeleteFragment`
- `ReplaceUserProfile`

### Full snapshot export
If you need to compare or debug:

```kotlin
val snapshotJson = core.exportMemorySnapshot()
```

## 6. Callback Example
```kotlin
core.setCallback(object : CoreCallback {
    override fun onEvent(event: CoreEvent) {
        when (event) {
            is CoreEvent.OnThinking -> {
                // show loading state
            }

            is CoreEvent.OnToolRequest -> {
                handleTool(event.id, event.toolName, event.argsJson)
            }

            is CoreEvent.OnFinish -> {
                event.memorySyncJson?.let { applyMemoryDelta(it) }
                val tasks = event.tasks
                val habits = event.habits
                val chat = event.chatResponse
            }

            is CoreEvent.OnError -> {
                // show error
            }
        }
    }
})
```

## 7. Pull-Based Fallback APIs
During early integration, Android can also read the latest engine output directly:

- `getLatestTasks()`
- `getLatestHabits()`
- `getLastMemorySyncJson()`
- `getLastFinishJson()`
- `getUserProfile()`
- `exportMemorySnapshot()`

These are useful for debugging and incremental rollout before the callback flow is fully wired into app state.

## 8. Tool Feedback
When Android finishes a local repository query:

```kotlin
scope.launch {
    val resultJson = """{"tasks":[]}"""
    core.submitToolResult(toolCallId, resultJson)
}
```
