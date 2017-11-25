extern crate clang_sys;
use clang_sys::*;
use std::ffi::CStr;

fn cxstring_to_string_leaky(s: CXString) -> String {
    if s.data.is_null() {
        return "".to_owned();
    }
    let c_str = unsafe { CStr::from_ptr(clang_getCString(s) as *const _) };
    c_str.to_string_lossy().into_owned()
}

fn cxstring_into_string(s: CXString) -> String {
    let ret = cxstring_to_string_leaky(s);
    unsafe { clang_disposeString(s) };
    ret
}

fn extract_clang_version() -> String {
    unsafe { cxstring_into_string(clang_getClangVersion()) }
}

fn main() {
    if !clang_sys::is_loaded() {
        clang_sys::load().unwrap();
    }
    let version = extract_clang_version();
    println!("{}", version);
}
