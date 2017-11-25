extern crate clang_sys;

fn main() {
    if !clang_sys::is_loaded() {
        clang_sys::load().unwrap();
    }
    unsafe { clang_sys::clang_createIndex(0, 1) };
    println!("Did I survive?");
}
