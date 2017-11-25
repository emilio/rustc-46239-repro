use std::{ptr, marker, mem, ops};
use std::os::raw::{self, c_int, c_void};

pub struct imp_Symbol<T> {
    pointer: *mut raw::c_void,
    pd: marker::PhantomData<T>
}

pub struct Symbol<'lib, T: 'lib> {
    inner: imp_Symbol<T>,
    pd: marker::PhantomData<&'lib T>
}

impl<'lib, T> ops::Deref for Symbol<'lib, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { mem::transmute(&self.inner.pointer) }
    }
}

extern fn local_clang_createIndex(_: raw::c_int, _: raw::c_int) -> *mut raw::c_void {
    println!("Reached!");
    ptr::null_mut()
}

struct Dummy;
impl Dummy {
    fn get<T>(&self) -> Result<Symbol<T>, ()> {
        Ok(Symbol {
            inner: imp_Symbol {
                pointer: local_clang_createIndex as *mut raw::c_void,
                pd: marker::PhantomData,
            },
            pd: marker::PhantomData,
        })
    }
}

#[derive(Debug, Default)]
pub struct Functions {
    pub clang_createIndex: Option<unsafe extern fn(_: c_int, _: c_int) -> CXIndex>,
}

/// A dynamically loaded instance of the `libclang` library.
pub struct SharedLibrary {
    pub functions: Functions,
}

pub fn load_manually() -> SharedLibrary {
    let mut functions = Functions::default();

    {
        let dummy = Dummy;
        let symbol = dummy.get().ok();
        functions.clang_createIndex = symbol.map(|s| *s);
    }

    SharedLibrary { functions }
}

pub type CXIndex = *mut c_void;

fn main() {
    let lib = load_manually();
    let fun = lib.functions.clang_createIndex.unwrap();
    unsafe { fun(0, 1) };
    println!("Did I survive?");
}
