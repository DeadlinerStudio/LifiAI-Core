# Deadliner AI Core - HarmonyOS Integration Guide

## 1. Current Strategy
Harmony should keep using a dedicated wrapper crate, not share UniFFI glue with iOS/Android.

Current workspace split:

- `core_logic`: pure engine
- `ffi_uniffi`: iOS / Android boundary
- `ffi_ohos`: Harmony boundary

That separation is intentional. It avoids mixing UniFFI and Harmony-specific binding macros in one crate.

## 2. Recommended External Usage
In another Harmony project directory:

```bash
git submodule add <your-github-url>/deadliner_core.git vendor/deadliner_core
cd vendor/deadliner_core
./scripts/build_ohos.sh
```

Recommended long-term pattern:

- Keep this repo as a separate engine repository
- Build artifacts in CI or locally
- Consume the generated Harmony library from the app project

## 3. Ownership Model
Harmony owns durable storage.
Rust owns orchestration and memory reasoning.

Recommended native stores:

- RelationalStore / Preferences / custom persistence for tasks, habits, and memory
- Rust only keeps a synchronized mirror during active engine sessions

## 4. Memory Sync Protocol
### Startup hydration
At startup or when the app resumes with fresher data, send a canonical snapshot into Rust.

Snapshot JSON shape:

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

### Delta write-back
When Rust finishes a turn, inspect `memorySyncJson`.

Apply it with this rule:

1. If `baseRevision` matches native revision, apply delta operations.
2. Commit transaction and set revision to `nextRevision`.
3. If revision mismatch exists, rebuild full snapshot and hydrate Rust again.

Delta operations:

- `UpsertFragment`
- `DeleteFragment`
- `ReplaceUserProfile`

### Full export
Rust can also export its current mirror snapshot for diagnostics or recovery:

```text
core.exportMemorySnapshot()
```

## 5. Harmony Tool Flow
When Rust emits a tool request:

1. Parse `argsJson`
2. Query native repo / RelationalStore
3. Serialize the result DTO to JSON
4. Call `submitToolResult(id, resultJson)`

When Rust emits `OnFinish`:

1. Apply tasks / habits to UI state
2. Persist `memorySyncJson` if present

## 6. Pull-Based Fallback APIs
For staged integration, expose wrapper methods corresponding to:

- `getLatestTasks()`
- `getLatestHabits()`
- `getLastMemorySyncJson()`
- `getLastFinishJson()`
- `getUserProfile()`
- `exportMemorySnapshot()`

These make it easier to debug the Rust engine before event-driven state handling is fully in place.

## 7. Next Binding Step
`ffi_ohos` is still a platform wrapper shell today.

The next implementation step is to add Harmony-specific exports only inside:

[ffi_ohos/src/lib.rs](/Users/aritxonly/Codes/Agent/deadliner_core/ffi_ohos/src/lib.rs)

Do not move Harmony binding macros into `core_logic`.
