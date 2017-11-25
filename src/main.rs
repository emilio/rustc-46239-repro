extern crate glob;
extern crate libloading;

/// The set of functions loaded dynamically.
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

impl SharedLibrary {
    //- Constructors -----------------------------

    fn new(library: libloading::Library) -> SharedLibrary {
        SharedLibrary { library: library, functions: Functions::default() }
    }
}

mod load {
    pub fn clang_createIndex(library: &mut super::SharedLibrary) {
        let symbol = unsafe { library.library.get(b"clang_createIndex") }.ok();
        library.functions.clang_createIndex = symbol.map(|s| *s);
    }
}

/// Loads a `libclang` shared library and returns the library instance.
///
/// This function does not attempt to load any functions from the shared library. The caller
/// is responsible for loading the functions they require.
///
/// # Failures
///
/// * a `libclang` shared library could not be found
/// * the `libclang` shared library could not be opened
pub fn load_manually() -> Result<SharedLibrary, String> {
    let file = "./target/debug/deps/libshared_lib.so";
    let library = libloading::Library::new(file).map_err(|_| {
        format!("the `libclang` shared library could not be opened: {}", file)
    });
    let mut library = SharedLibrary::new(try!(library));
    load::clang_createIndex(&mut library);
    Ok(library)
}

use std::os::raw::{c_int, c_void};

// Opaque ________________________________________

pub type CXIndex = *mut c_void;

fn main() {
    let lib = load_manually().unwrap();
    let fun = lib.functions.clang_createIndex.unwrap();
    unsafe { fun(0, 1) };
    println!("Did I survive?");
}
