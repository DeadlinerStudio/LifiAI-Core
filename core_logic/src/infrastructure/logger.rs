#[cfg(target_os = "android")]
use std::ffi::CString;
#[cfg(target_env = "ohos")]
use std::ffi::CString;

#[cfg(any(target_os = "android", target_env = "ohos"))]
const LOG_TAG: &str = "DeadlinerCore";

pub fn debug(message: &str) {
    log_debug(message);
}

fn log_debug(message: &str) {
    #[cfg(target_os = "android")]
    {
        android_log_debug(message);
    }

    #[cfg(target_env = "ohos")]
    {
        ohos_log_debug(message);
    }

    #[cfg(not(any(target_os = "android", target_env = "ohos")))]
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

#[cfg(target_env = "ohos")]
fn ohos_log_debug(message: &str) {
    let tag = cstring_sanitized(LOG_TAG);
    let format = cstring_sanitized("%{public}s");
    let text = cstring_sanitized(message);

    unsafe {
        OH_LOG_Print(
            OHOS_LOG_TYPE_APP,
            OHOS_LOG_LEVEL_DEBUG,
            OHOS_LOG_DOMAIN,
            tag.as_ptr(),
            format.as_ptr(),
            text.as_ptr(),
        );
    }
}

#[cfg(target_env = "ohos")]
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

#[cfg(target_env = "ohos")]
const OHOS_LOG_TYPE_APP: i32 = 0;
#[cfg(target_env = "ohos")]
const OHOS_LOG_LEVEL_DEBUG: i32 = 3;
#[cfg(target_env = "ohos")]
const OHOS_LOG_DOMAIN: u32 = 0xD001100;

#[cfg(target_env = "ohos")]
#[link(name = "hilog_ndk.z")]
unsafe extern "C" {
    fn OH_LOG_Print(
        log_type: i32,
        log_level: i32,
        domain: u32,
        tag: *const std::ffi::c_char,
        fmt: *const std::ffi::c_char,
        ...
    ) -> i32;
}
