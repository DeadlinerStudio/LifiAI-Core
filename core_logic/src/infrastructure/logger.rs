#[cfg(target_os = "android")]
use std::ffi::CString;

#[cfg(target_os = "android")]
const LOG_TAG: &str = "DeadlinerCore";

pub fn debug(message: &str) {
    log_debug(message);
}

fn log_debug(message: &str) {
    #[cfg(target_os = "android")]
    {
        android_log_debug(message);
    }

    #[cfg(not(target_os = "android"))]
    {
        println!("[DEBUG] {}", message);
    }
}

#[cfg(target_os = "android")]
fn android_log_debug(message: &str) {
    let tag = cstring_sanitized(LOG_TAG);
    let text = cstring_sanitized(message);

    unsafe {
        __android_log_write(3, tag.as_ptr(), text.as_ptr()); // ANDROID_LOG_DEBUG
    }
}

#[cfg(target_os = "android")]
fn cstring_sanitized(input: &str) -> CString {
    let mut bytes = input.as_bytes().to_vec();
    for byte in &mut bytes {
        if *byte == 0 {
            *byte = b' ';
        }
    }
    CString::new(bytes).expect("sanitized string cannot contain interior NUL")
}

#[cfg(target_os = "android")]
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        prio: i32,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> i32;
}
