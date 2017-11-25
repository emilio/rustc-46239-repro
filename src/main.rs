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
    let file = try!(build::find_shared_library());
    let library = libloading::Library::new(&file).map_err(|_| {
        format!("the `libclang` shared library could not be opened: {}", file.display())
    });
    let mut library = SharedLibrary::new(try!(library));
    load::clang_createIndex(&mut library);
    Ok(library)
}

use std::mem;

use std::os::raw::{c_int, c_void};

//================================================
// Structs
//================================================

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXVersion {
    pub Major: c_int,
    pub Minor: c_int,
    pub Subminor: c_int,
}

// Opaque ________________________________________

pub type CXIndex = *mut c_void;

mod build {
use std::env;
use std::fs::{self, File};
use std::io::{Read};
use std::path::{Path, PathBuf};
use std::process::{Command};

use glob::{self, MatchOptions};

/// Returns the components of the version appended to the supplied file.
fn parse_version(file: &Path) -> Vec<u32> {
    let string = file.to_str().unwrap_or("");
    let components = string.split('.').skip(2);
    components.map(|s| s.parse::<u32>().unwrap_or(0)).collect()
}

/// Returns a path to one of the supplied files if such a file can be found in the supplied directory.
fn contains(directory: &Path, files: &[String]) -> Option<PathBuf> {
    // Join the directory to the files to obtain our glob patterns.
    let patterns = files.iter().filter_map(|f| directory.join(f).to_str().map(ToOwned::to_owned));

    // Prevent wildcards from matching path separators.
    let mut options = MatchOptions::new();
    options.require_literal_separator = true;

    // Collect any files that match the glob patterns.
    let mut matches = patterns.flat_map(|p| {
        if let Ok(paths) = glob::glob_with(&p, &options) {
            paths.filter_map(Result::ok).collect()
        } else {
            vec![]
        }
    }).collect::<Vec<_>>();

    // Sort the matches by their version, preferring shorter and higher versions.
    matches.sort_by_key(|m| parse_version(m));
    matches.pop()
}

/// Runs a console command, returning the output if the command was successfully executed.
fn run(command: &str, arguments: &[&str]) -> Option<String> {
    Command::new(command).args(arguments).output().map(|o| {
        String::from_utf8_lossy(&o.stdout).into_owned()
    }).ok()
}

/// Runs `llvm-config`, returning the output if the command was successfully executed.
fn run_llvm_config(arguments: &[&str]) -> Result<String, String> {
    match run(&env::var("LLVM_CONFIG_PATH").unwrap_or_else(|_| "llvm-config".into()), arguments) {
        Some(output) => Ok(output),
        None => {
            let message = format!(
                "couldn't execute `llvm-config {}`, set the LLVM_CONFIG_PATH environment variable \
                to a path to a valid `llvm-config` executable",
                arguments.join(" "),
            );
            Err(message)
        },
    }
}

/// Backup search directory globs for FreeBSD and Linux.
const SEARCH_LINUX: &'static [&'static str] = &[
    "/usr/lib*",
    "/usr/lib*/*",
    "/usr/lib*/*/*",
    "/usr/local/lib*",
    "/usr/local/lib*/*",
    "/usr/local/lib*/*/*",
    "/usr/local/llvm*/lib",
];

/// Backup search directory globs for OS X.
const SEARCH_OSX: &'static [&'static str] = &[
    "/usr/local/opt/llvm*/lib",
    "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib",
    "/Library/Developer/CommandLineTools/usr/lib",
    "/usr/local/opt/llvm*/lib/llvm*/lib",
];

/// Backup search directory globs for Windows.
const SEARCH_WINDOWS: &'static [&'static str] = &[
    "C:\\LLVM\\lib",
    "C:\\Program Files*\\LLVM\\lib",
    "C:\\MSYS*\\MinGW*\\lib",
];

/// Indicates the type of library being searched for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Library {
    Dynamic,
    Static,
}

impl Library {
    /// Checks whether the supplied file is a valid library for the architecture.
    fn check(&self, file: &PathBuf) -> Result<(), String> {
        if cfg!(any(target_os="freebsd", target_os="linux")) {
            if *self == Library::Static {
                return Ok(());
            }
            let mut file = try!(File::open(file).map_err(|e| e.to_string()));
            let mut elf = [0; 5];
            try!(file.read_exact(&mut elf).map_err(|e| e.to_string()));
            if elf[..4] != [127, 69, 76, 70] {
                return Err("invalid ELF header".into());
            }
            if cfg!(target_pointer_width="32") && elf[4] != 1 {
                return Err("invalid ELF class (64-bit)".into());
            }
            if cfg!(target_pointer_width="64") && elf[4] != 2 {
                return Err("invalid ELF class (32-bit)".into());
            }
            Ok(())
        } else {
            Ok(())
        }
    }
}

/// Searches for a library, returning the directory it can be found in if the search was successful.
fn find(library: Library, files: &[String], env: &str) -> Result<PathBuf, String> {
    let mut skipped = vec![];

    /// Attempts to return the supplied file.
    macro_rules! try_file {
        ($file:expr) => ({
            match library.check(&$file) {
                Ok(_) => return Ok($file),
                Err(message) => skipped.push(format!("({}: {})", $file.display(), message)),
            }
        });
    }

    /// Searches the supplied directory and, on Windows, any relevant sibling directories.
    macro_rules! search_directory {
        ($directory:ident) => {
            if let Some(file) = contains(&$directory, files) {
                try_file!(file);
            }

            // On Windows, `libclang.dll` is usually found in the LLVM `bin` directory while
            // `libclang.lib` is usually found in the LLVM `lib` directory. To keep things
            // consistent with other platforms, only LLVM `lib` directories are included in the
            // backup search directory globs so we need to search the LLVM `bin` directory here.
            if cfg!(target_os="windows") && $directory.ends_with("lib") {
                let sibling = $directory.parent().unwrap().join("bin");
                if let Some(file) = contains(&sibling, files) {
                    try_file!(file);
                }
            }
        }
    }

    // Search the directory provided by the relevant environment variable if it is set.
    if let Ok(directory) = env::var(env).map(|d| Path::new(&d).to_path_buf()) {
        search_directory!(directory);
    }

    // Search the `bin` and `lib` subdirectories in the directory returned by
    // `llvm-config --prefix` if `llvm-config` is available.
    if let Ok(output) = run_llvm_config(&["--prefix"]) {
        let directory = Path::new(output.lines().next().unwrap()).to_path_buf();
        let bin = directory.join("bin");
        if let Some(file) = contains(&bin, files) {
            try_file!(file);
        }
        let lib = directory.join("lib");
        if let Some(file) = contains(&lib, files) {
            try_file!(file);
        }
    }

    // Search the backup directories.
    let search = if cfg!(any(target_os="freebsd", target_os="linux")) {
        SEARCH_LINUX
    } else if cfg!(target_os="macos") {
        SEARCH_OSX
    } else if cfg!(target_os="windows") {
        SEARCH_WINDOWS
    } else {
        &[]
    };
    for pattern in search {
        let mut options = MatchOptions::new();
        options.case_sensitive = false;
        options.require_literal_separator = true;
        if let Ok(paths) = glob::glob_with(pattern, &options) {
            for path in paths.filter_map(Result::ok).filter(|p| p.is_dir()) {
                search_directory!(path);
            }
        }
    }

    let message = format!(
        "couldn't find any of [{}], set the {} environment variable to a path where one of these \
         files can be found (skipped: [{}])",
        files.iter().map(|f| format!("'{}'", f)).collect::<Vec<_>>().join(", "),
        env,
        skipped.join(", "),
    );
    Err(message)
}

/// Searches for a `libclang` shared library, returning the path to such a shared library if the
/// search was successful.
pub fn find_shared_library() -> Result<PathBuf, String> {
    let mut files = vec![format!("{}clang{}", env::consts::DLL_PREFIX, env::consts::DLL_SUFFIX)];
    if cfg!(any(target_os="freebsd", target_os="linux", target_os="openbsd")) {
        // Some BSDs and Linux distributions don't create a `libclang.so` symlink, so we need to
        // look for any versioned files (e.g., `libclang.so.3.9`).
        files.push("libclang.so.*".into());
    }
    if cfg!(target_os="windows") {
        // The official LLVM build uses `libclang.dll` on Windows instead of `clang.dll`. However,
        // unofficial builds such as MinGW use `clang.dll`.
        files.push("libclang.dll".into());
    }
    find(Library::Dynamic, &files, "LIBCLANG_PATH")
}

}

fn main() {
    let lib = load_manually().unwrap();
    let fun = lib.functions.clang_createIndex.unwrap();
    unsafe { fun(0, 1) };
    println!("Did I survive?");
}
