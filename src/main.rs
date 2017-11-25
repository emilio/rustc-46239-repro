use std::{ptr, marker, mem, ops};
use std::os::raw;

pub struct Library {
    whatever: *mut (),
}

pub struct imp_Symbol<T> {
    pointer: *mut raw::c_void,
    pd: marker::PhantomData<T>
}

impl<T> ops::Deref for imp_Symbol<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe {
            // Additional reference level for a dereference on `deref` return value.
            mem::transmute(&self.pointer)
        }
    }
}

pub struct Symbol<'lib, T: 'lib> {
    inner: imp_Symbol<T>,
    pd: marker::PhantomData<&'lib T>
}

impl<'lib, T> ops::Deref for Symbol<'lib, T> {
    type Target = T;
    fn deref(&self) -> &T {
        ops::Deref::deref(&self.inner)
    }
}

extern fn local_clang_createIndex(_: raw::c_int, _: raw::c_int) -> *mut raw::c_void {
    println!("Reached!");
    ptr::null_mut()
}

impl Library {
    pub fn new(_: &str) -> Result<Self, ()> {
        Ok(Self { whatever: ptr::null_mut(), })
    }

    pub fn get<T>(&self, _: &[u8]) -> Result<Symbol<T>, ()> {
        Ok(Symbol {
            inner: imp_Symbol {
                pointer: local_clang_createIndex as *mut raw::c_void,
                pd: marker::PhantomData,
            },
            pd: marker::PhantomData,
        })
    }
}

use std::os::raw::{c_int, c_void};

#[derive(Debug, Default)]
pub struct Functions {
    pub clang_createIndex: Option<unsafe extern fn(_: c_int, _: c_int) -> CXIndex>,
}

/// A dynamically loaded instance of the `libclang` library.
pub struct SharedLibrary {
    library: Library,
    pub functions: Functions,
}

pub fn load_manually() -> SharedLibrary {
    let file = "./target/debug/deps/libshared_lib.so";
    let library = Library::new(file).unwrap();
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
