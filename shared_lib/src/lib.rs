
use std::os::raw::{c_int, c_void};
use std::ptr;

#[no_mangle]
pub extern fn clang_createIndex(_: c_int, _: c_int) -> *mut c_void {
    println!("Reached!");
    ptr::null_mut()
}
