pub use core_logic::interfaces::events::{CoreEvent, FFIHabit, FFITask};
pub use core_logic::interfaces::ffi_handlers::{extract_tasks, CoreCallback, CoreError, DeadlinerCore};

// This crate is the future OHOS/NAPI boundary. Keep platform-specific glue here
// and treat `core_logic` as a pure internal engine.
