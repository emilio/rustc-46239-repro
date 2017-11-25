use std::{ptr, marker, mem, ops};
use std::os::raw::{self, c_int, c_void};

pub struct Symbol<'lib, T: 'lib> {
    pointer: *mut raw::c_void,
    pd: marker::PhantomData<&'lib T>
}

impl<'lib, T> ops::Deref for Symbol<'lib, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { mem::transmute(&self.pointer) }
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
            pointer: local_clang_createIndex as *mut raw::c_void,
            pd: marker::PhantomData,
        })
    }
}

#[derive(Debug, Default)]
pub struct Functions {
    pub clang_createIndex: Option<unsafe extern fn(_: c_int, _: c_int) -> CXIndex>,
}

pub fn load_manually() -> Functions {
    let mut functions = Functions::default();

    {
        let dummy = Dummy;
        let symbol = dummy.get().ok();
        functions.clang_createIndex = symbol.map(|s| *s);
    }

    functions
}

pub type CXIndex = *mut c_void;

fn main() {
    let lib = load_manually();
    let fun = lib.clang_createIndex.unwrap();
    unsafe { fun(0, 1) };
    println!("Did I survive?");
}
