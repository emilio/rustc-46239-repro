extern crate glob;
extern crate libc;
extern crate libloading;

pub mod support {

use std::{io, env};
use std::process::{Command};
use std::path::{Path, PathBuf};

use glob;

use libc::{c_int};

use super::{CXVersion};

//================================================
// Macros
//================================================

// try_opt! ______________________________________

macro_rules! try_opt {
    ($option:expr) => ({
        match $option {
            Some(some) => some,
            None => return None,
        }
    });
}

//================================================
// Structs
//================================================

/// A `clang` executable.
#[derive(Clone, Debug)]
pub struct Clang {
    /// The path to this `clang` executable.
    pub path: PathBuf,
    /// The version of this `clang` executable if it could be parsed.
    pub version: Option<CXVersion>,
    /// The directories searched by this `clang` executable for C headers if they could be parsed.
    pub c_search_paths: Option<Vec<PathBuf>>,
    /// The directories searched by this `clang` executable for C++ headers if they could be parsed.
    pub cpp_search_paths: Option<Vec<PathBuf>>,
}

impl Clang {
    //- Constructors -----------------------------

    fn new(path: PathBuf, args: &[String]) -> Clang {
        let version = parse_version(&path);
        let c_search_paths = parse_search_paths(&path, "c", args);
        let cpp_search_paths = parse_search_paths(&path, "c++", args);
        Clang {
            path: path,
            version: version,
            c_search_paths: c_search_paths,
            cpp_search_paths: cpp_search_paths,
        }
    }

    /// Returns a `clang` executable if one can be found.
    ///
    /// If the `CLANG_PATH` environment variable is set, that is the instance of `clang` used.
    /// Otherwise, a series of directories are searched. First, If a path is supplied, that is the
    /// first directory searched. Then, the directory returned by `llvm-config --bindir` is
    /// searched. On OS X systems, `xcodebuild -find clang` will next be queried. Last, the
    /// directories in the system's `PATH` are searched.
    pub fn find(path: Option<&Path>, args: &[String]) -> Option<Clang> {
        if let Ok(path) = env::var("CLANG_PATH") {
            return Some(Clang::new(path.into(), args));
        }

        let mut paths = vec![];
        if let Some(path) = path {
            paths.push(path.into());
        }
        if let Ok(path) = run_llvm_config(&["--bindir"]) {
            paths.push(path.into());
        }
        if cfg!(target_os="macos") {
            if let Ok((path, _)) = run("xcodebuild", &["-find", "clang"]) {
                paths.push(path.into());
            }
        }
        paths.extend(env::split_paths(&env::var("PATH").unwrap()));

        let default = format!("clang{}", env::consts::EXE_SUFFIX);
        let versioned = format!("clang-[0-9]*{}", env::consts::EXE_SUFFIX);
        let patterns = &[&default[..], &versioned[..]];
        for path in paths {
            if let Some(path) = find(&path, patterns) {
                return Some(Clang::new(path, args));
            }
        }
        None
    }
}

//================================================
// Functions
//================================================

/// Returns the first match to the supplied glob patterns in the supplied directory if there are any
/// matches.
fn find(directory: &Path, patterns: &[&str]) -> Option<PathBuf> {
    for pattern in patterns {
        let pattern = directory.join(pattern).to_string_lossy().into_owned();
        if let Some(path) = try_opt!(glob::glob(&pattern).ok()).filter_map(|p| p.ok()).next() {
            if path.is_file() && is_executable(&path).unwrap_or(false) {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> io::Result<bool> {
    use libc;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())?;
    unsafe { Ok(libc::access(path.as_ptr(), libc::X_OK) == 0) }
}

#[cfg(not(unix))]
fn is_executable(_: &Path) -> io::Result<bool> {
    Ok(true)
}

/// Attempts to run an executable, returning the `stdout` and `stderr` output if successful.
fn run(executable: &str, arguments: &[&str]) -> Result<(String, String), String> {
    Command::new(executable).args(arguments).output().map(|o| {
        let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
        (stdout, stderr)
    }).map_err(|_| format!("could not run executable: `{}`", executable))
}

/// Runs `clang`, returning the `stdout` and `stderr` output.
fn run_clang(path: &Path, arguments: &[&str]) -> (String, String) {
    run(&path.to_string_lossy().into_owned(), arguments).unwrap()
}

/// Runs `llvm-config`, returning the `stdout` output if successful.
fn run_llvm_config(arguments: &[&str]) -> Result<String, String> {
    let config = env::var("LLVM_CONFIG_PATH").unwrap_or_else(|_| "llvm-config".to_string());
    run(&config, arguments).map(|(o, _)| o)
}

/// Parses a version number if possible, ignoring trailing non-digit characters.
fn parse_version_number(number: &str) -> Option<c_int> {
    number.chars().take_while(|c| c.is_digit(10)).collect::<String>().parse().ok()
}

/// Parses the version from the output of a `clang` executable if possible.
fn parse_version(path: &Path) -> Option<CXVersion> {
    let output = run_clang(path, &["--version"]).0;
    let start = try_opt!(output.find("version ")) + 8;
    let mut numbers = try_opt!(output[start..].split_whitespace().nth(0)).split('.');
    let major = try_opt!(numbers.next().and_then(parse_version_number));
    let minor = try_opt!(numbers.next().and_then(parse_version_number));
    let subminor = numbers.next().and_then(parse_version_number).unwrap_or(0);
    Some(CXVersion { Major: major, Minor: minor, Subminor: subminor })
}

/// Parses the search paths from the output of a `clang` executable if possible.
fn parse_search_paths(path: &Path, language: &str, args: &[String]) -> Option<Vec<PathBuf>> {
    let mut clang_args = vec!["-E", "-x", language, "-", "-v"];
    clang_args.extend(args.iter().map(|s| &**s));
    let output = run_clang(path, &clang_args).1;
    let start = try_opt!(output.find("#include <...> search starts here:")) + 34;
    let end = try_opt!(output.find("End of search list."));
    let paths = output[start..end].replace("(framework directory)", "");
    Some(paths.lines().filter(|l| !l.is_empty()).map(|l| Path::new(l.trim()).into()).collect())
}
}

macro_rules! link {
    (@LOAD: #[cfg($cfg:meta)] fn $name:ident($($pname:ident: $pty:ty), *) $(-> $ret:ty)*) => (
        #[cfg($cfg)]
        pub fn $name(library: &mut super::SharedLibrary) {
            let symbol = unsafe { library.library.get(stringify!($name).as_bytes()) }.ok();
            library.functions.$name = symbol.map(|s| *s);
        }

        #[cfg(not($cfg))]
        pub fn $name(_: &mut super::SharedLibrary) {}
    );

    (@LOAD: fn $name:ident($($pname:ident: $pty:ty), *) $(-> $ret:ty)*) => (
        link!(@LOAD: #[cfg(any(feature="runtime", not(feature="runtime")))] fn $name($($pname: $pty), *) $(-> $ret)*);
    );

    ($($(#[cfg($cfg:meta)])* pub fn $name:ident($($pname:ident: $pty:ty), *) $(-> $ret:ty)*;)+) => (
        use std::cell::{RefCell};
        use std::sync::{Arc};

        /// The set of functions loaded dynamically.
        #[derive(Debug, Default)]
        pub struct Functions {
            $($(#[cfg($cfg)])* pub $name: Option<unsafe extern fn($($pname: $pty), *) $(-> $ret)*>,)+
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

        thread_local!(static LIBRARY: RefCell<Option<Arc<SharedLibrary>>> = RefCell::new(None));

        /// Returns whether a `libclang` shared library is loaded on this thread.
        pub fn is_loaded() -> bool {
            LIBRARY.with(|l| l.borrow().is_some())
        }

        fn with_library<T, F>(f: F) -> Option<T> where F: FnOnce(&SharedLibrary) -> T {
            LIBRARY.with(|l| {
                match l.borrow().as_ref() {
                    Some(library) => Some(f(&library)),
                    _ => None,
                }
            })
        }

        $(
            $(#[cfg($cfg)])*
            pub unsafe fn $name($($pname: $pty), *) $(-> $ret)* {
                let f = with_library(|l| {
                    match l.functions.$name {
                        Some(f) => f,
                        _ => panic!(concat!("function not loaded: ", stringify!($name))),
                    }
                }).expect("a `libclang` shared library is not loaded on this thread");
                f($($pname), *)
            }

            $(#[cfg($cfg)])*
            pub mod $name {
                pub fn is_loaded() -> bool {
                    super::with_library(|l| l.functions.$name.is_some()).unwrap_or(false)
                }
            }
        )+

        mod load {
            $(link!(@LOAD: $(#[cfg($cfg)])* fn $name($($pname: $pty), *) $(-> $ret)*);)+
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
            $(load::$name(&mut library);)+
            Ok(library)
        }

        /// Loads a `libclang` shared library for use in the current thread.
        ///
        /// This functions attempts to load all the functions in the shared library. Whether a
        /// function has been loaded can be tested by calling the `is_loaded` function on the
        /// module with the same name as the function (e.g., `clang_createIndex::is_loaded()` for
        /// the `clang_createIndex` function).
        ///
        /// # Failures
        ///
        /// * a `libclang` shared library could not be found
        /// * the `libclang` shared library could not be opened
        #[allow(dead_code)]
        pub fn load() -> Result<(), String> {
            let library = Arc::new(try!(load_manually()));
            LIBRARY.with(|l| *l.borrow_mut() = Some(library));
            Ok(())
        }

        /// Unloads the `libclang` shared library in use in the current thread.
        ///
        /// # Failures
        ///
        /// * a `libclang` shared library is not in use in the current thread
        pub fn unload() -> Result<(), String> {
            let library = set_library(None);
            if library.is_some() {
                Ok(())
            } else {
                Err("a `libclang` shared library is not in use in the current thread".into())
            }
        }

        /// Returns the library instance stored in TLS.
        ///
        /// This functions allows for sharing library instances between threads.
        pub fn get_library() -> Option<Arc<SharedLibrary>> {
            LIBRARY.with(|l| l.borrow_mut().clone())
        }

        /// Sets the library instance stored in TLS and returns the previous library.
        ///
        /// This functions allows for sharing library instances between threads.
        pub fn set_library(library: Option<Arc<SharedLibrary>>) -> Option<Arc<SharedLibrary>> {
            LIBRARY.with(|l| mem::replace(&mut *l.borrow_mut(), library))
        }
    )
}

use std::mem;

use libc::{c_char, c_int, c_longlong, c_uint, c_ulong, c_ulonglong, c_void, time_t};
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

macro_rules! opaque { ($name:ident) => (pub type $name = *mut c_void;); }

opaque!(CXIndex);

link! {
    pub fn clang_createIndex(exclude: c_int, display: c_int) -> CXIndex;
}

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

/// Returns the name of an LLVM or Clang library from a path to such a library.
fn get_library_name(path: &Path) -> Option<String> {
    path.file_stem().map(|p| {
        let string = p.to_string_lossy();
        if string.starts_with("lib") {
            string[3..].to_owned()
        } else {
            string.to_string()
        }
    })
}

/// Returns the LLVM libraries required to link to `libclang` statically.
fn get_llvm_libraries() -> Vec<String> {
    run_llvm_config(&["--libs"]).unwrap().split_whitespace().filter_map(|p| {
        // Depending on the version of `llvm-config` in use, listed libraries may be in one of two
        // forms, a full path to the library or simply prefixed with `-l`.
        if p.starts_with("-l") {
            Some(p[2..].into())
        } else {
            get_library_name(Path::new(p))
        }
    }).collect()
}

/// Clang libraries required to link to `libclang` 3.5 and later statically.
const CLANG_LIBRARIES: &'static [&'static str] = &[
    "clang",
    "clangAST",
    "clangAnalysis",
    "clangBasic",
    "clangDriver",
    "clangEdit",
    "clangFrontend",
    "clangIndex",
    "clangLex",
    "clangParse",
    "clangRewrite",
    "clangSema",
    "clangSerialization",
];

/// Returns the Clang libraries required to link to `libclang` statically.
fn get_clang_libraries<P: AsRef<Path>>(directory: P) -> Vec<String> {
    let pattern = directory.as_ref().join("libclang*.a").to_string_lossy().to_string();
    if let Ok(libraries) = glob::glob(&pattern) {
        libraries.filter_map(|l| l.ok().and_then(|l| get_library_name(&l))).collect()
    } else {
        CLANG_LIBRARIES.iter().map(|l| l.to_string()).collect()
    }
}
}

fn main() {
    if !is_loaded() {
        load().unwrap();
    }
    unsafe { clang_createIndex(0, 1) };
    println!("Did I survive?");
}
