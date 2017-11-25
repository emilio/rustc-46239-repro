extern crate libloading;

use std::os::raw::{c_int, c_void};

#[derive(Debug, Default)]
pub struct Functions {
    pub clang_createIndex: Option<unsafe extern fn(_: c_int, _: c_int) -> CXIndex>,
}

/// A dynamically loaded instance of the `libclang` library.
#[derive(Debug)]
pub struct SharedLibrary {
    library: libloading::Library,
    pub functions: Functions,
}

pub fn load_manually() -> SharedLibrary {
    let file = "./target/debug/deps/libshared_lib.so";
    let library = libloading::Library::new(file).unwrap();
    let mut functions = Functions::default();

    {
        let symbol = unsafe { library.get(b"clang_createIndex") }.ok();
        functions.clang_createIndex = symbol.map(|s| *s);
    }

    SharedLibrary { library, functions }
}

pub type CXIndex = *mut c_void;

fn main() {
    let lib = load_manually();
    let fun = lib.functions.clang_createIndex.unwrap();
    unsafe { fun(0, 1) };
    println!("Did I survive?");
}
