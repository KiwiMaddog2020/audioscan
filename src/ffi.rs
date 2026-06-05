//! C ABI entry points for audioscan.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::{ScanConfig, analyze_path};

const VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

/// Analyze an audio file and return the existing audioscan JSON contract.
///
/// Returns a newly allocated NUL-terminated JSON string on success. The caller
/// owns the returned pointer and must release it with
/// [`audioscan_string_free`]. Returns null when `path` is null, `path` is not
/// valid UTF-8, the scan configuration is invalid, analysis fails, JSON
/// serialization fails, or a Rust panic is caught.
///
/// # Safety
/// `path` must be either null or a valid pointer to a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn audioscan_analyze_json(
    path: *const c_char,
    threshold_db: f64,
    min_gap_sec: f64,
    strict: c_int,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        analyze_json(path, threshold_db, min_gap_sec, strict).unwrap_or(ptr::null_mut())
    }))
    .unwrap_or(ptr::null_mut())
}

/// Free a string returned by [`audioscan_analyze_json`].
///
/// Passing null is a safe no-op.
///
/// # Safety
/// `s` must be null or a pointer previously returned by
/// [`audioscan_analyze_json`] that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn audioscan_string_free(s: *mut c_char) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !s.is_null() {
            // SAFETY: Callers must pass only pointers returned by CString::into_raw
            // from audioscan_analyze_json, and only once.
            drop(unsafe { CString::from_raw(s) });
        }
    }));
}

/// Return audioscan's static package version string.
///
/// The returned pointer has static storage duration and must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn audioscan_version() -> *const c_char {
    catch_unwind(|| VERSION.as_ptr().cast::<c_char>())
        .unwrap_or_else(|_| VERSION.as_ptr().cast::<c_char>())
}

fn analyze_json(
    path: *const c_char,
    threshold_db: f64,
    min_gap_sec: f64,
    strict: c_int,
) -> Option<*mut c_char> {
    if path.is_null() {
        return None;
    }

    // SAFETY: The extern function's safety contract requires a valid
    // NUL-terminated C string when path is non-null.
    let path = unsafe { CStr::from_ptr(path) }.to_str().ok()?;
    let config = ScanConfig {
        threshold_db,
        min_gap_sec,
        strict: strict != 0,
        max_decode_secs: None,
    };
    let analysis = analyze_path(path, &config).ok()?;
    let json = serde_json::to_string(&analysis).ok()?;
    let c_json = CString::new(json).ok()?;

    Some(c_json.into_raw())
}
