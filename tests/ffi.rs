use std::ffi::{CStr, CString};
use std::ptr;

use serde_json::Value;

#[test]
fn analyze_json_returns_parseable_contract() {
    let path = CString::new("tests/fixtures/tone_1khz_1s.flac").expect("fixture path has no NUL");

    // SAFETY: path is a valid NUL-terminated C string for the duration of the call.
    let json_ptr = unsafe { audioscan::ffi::audioscan_analyze_json(path.as_ptr(), -30.0, 5.0, 0) };
    assert!(!json_ptr.is_null(), "analysis should succeed");

    // SAFETY: audioscan_analyze_json returned a valid NUL-terminated string on success.
    let json = unsafe { CStr::from_ptr(json_ptr) }
        .to_str()
        .expect("audioscan JSON is UTF-8")
        .to_owned();
    assert!(json.contains("\"schema_version\""));

    // SAFETY: json_ptr came from audioscan_analyze_json and has not been freed.
    unsafe { audioscan::ffi::audioscan_string_free(json_ptr) };

    let value: Value = serde_json::from_str(&json).expect("audioscan JSON parses");
    assert!(value.get("schema_version").is_some());
}

#[test]
fn analyze_json_null_path_returns_null() {
    // SAFETY: null is an explicitly supported no-op input for the path pointer.
    let json_ptr = unsafe { audioscan::ffi::audioscan_analyze_json(ptr::null(), -30.0, 5.0, 0) };
    assert!(json_ptr.is_null());
}
