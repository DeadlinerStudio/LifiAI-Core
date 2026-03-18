pub use core_logic::interfaces::events::{CoreEvent, FFIHabit, FFITask};
pub use core_logic::interfaces::ffi_handlers::{extract_tasks, CoreCallback, CoreError, DeadlinerCore};

// This crate is the future UniFFI boundary. Keep business logic in `core_logic`
// and add UniFFI-specific exports here when the binding layer is reintroduced.
