# Deadliner AI Core - Android Integration Guide

## 1. Build and Link
The Rust core is compiled into `.so` files for different ABIs (arm64, x86_64) and a Kotlin binding file.

## 2. Usage in Kotlin/Compose

### Initialization
```kotlin
val storagePath = context.filesDir.absolutePath
val core = DeadlinerCore(
    apiKey = "your-key",
    baseUrl = "https://api.deepseek.com/v1",
    modelId = "deepseek-chat",
    storagePath = storagePath
)

core.setCallback(object : CoreCallback {
    override fun onEvent(event: CoreEvent) {
        when (event) {
            is CoreEvent.OnThinking -> { /* Show Loader */ }
            is CoreEvent.OnFinish -> {
                val tasks = event.tasks
                // Update State
            }
            // ...
        }
    }
})
```

### Processing Input
```kotlin
// Coroutine compatible if using uniffi-rs async
scope.launch {
    core.processInput("Remind me to study at 8pm")
}
```
