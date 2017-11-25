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

pub type CXClientData = *mut c_void;
pub type CXCursorVisitor = extern fn(CXCursor, CXCursor, CXClientData) -> CXChildVisitResult;
pub type CXInclusionVisitor = extern fn(CXFile, *mut CXSourceLocation, c_uint, CXClientData);

/// Defines a C enum as a series of constants.
macro_rules! cenum {
    ($(#[$meta:meta])* enum $name:ident {
        $($(#[$vmeta:meta])* const $variant:ident = $value:expr), +,
    }) => (
        pub type $name = c_int;

        $($(#[$vmeta])* pub const $variant: $name = $value;)+
    );
    ($(#[$meta:meta])* enum $name:ident {
        $($(#[$vmeta:meta])* const $variant:ident = $value:expr); +;
    }) => (
        pub type $name = c_int;

        $($(#[$vmeta])* pub const $variant: $name = $value;)+
    );
}

// default! ______________________________________

/// Implements a zeroing implementation of `Default` for the supplied type.
macro_rules! default {
    (#[$meta:meta] $ty:ty) => {
        #[$meta]
        impl Default for $ty {
            fn default() -> $ty {
                unsafe { mem::zeroed() }
            }
        }
    };

    ($ty:ty) => {
        impl Default for $ty {
            fn default() -> $ty {
                unsafe { mem::zeroed() }
            }
        }
    };
}

//================================================
// Enums
//================================================

cenum! {
    enum CXAvailabilityKind {
        const CXAvailability_Available = 0,
        const CXAvailability_Deprecated = 1,
        const CXAvailability_NotAvailable = 2,
        const CXAvailability_NotAccessible = 3,
    }
}

cenum! {
    enum CXCallingConv {
        const CXCallingConv_Default = 0,
        const CXCallingConv_C = 1,
        const CXCallingConv_X86StdCall = 2,
        const CXCallingConv_X86FastCall = 3,
        const CXCallingConv_X86ThisCall = 4,
        const CXCallingConv_X86Pascal = 5,
        const CXCallingConv_AAPCS = 6,
        const CXCallingConv_AAPCS_VFP = 7,
        /// Only produced by `libclang` 4.0 and later.
        const CXCallingConv_X86RegCall = 8,
        const CXCallingConv_IntelOclBicc = 9,
        const CXCallingConv_Win64 = 10,
        const CXCallingConv_X86_64Win64 = 10,
        const CXCallingConv_X86_64SysV = 11,
        /// Only produced by `libclang` 3.6 and later.
        const CXCallingConv_X86VectorCall = 12,
        /// Only produced by `libclang` 3.9 and later.
        const CXCallingConv_Swift = 13,
        /// Only produced by `libclang` 3.9 and later.
        const CXCallingConv_PreserveMost = 14,
        /// Only produced by `libclang` 3.9 and later.
        const CXCallingConv_PreserveAll = 15,
        const CXCallingConv_Invalid = 100,
        const CXCallingConv_Unexposed = 200,
    }
}

cenum! {
    enum CXChildVisitResult {
        const CXChildVisit_Break = 0,
        const CXChildVisit_Continue = 1,
        const CXChildVisit_Recurse = 2,
    }
}

cenum! {
    enum CXCommentInlineCommandRenderKind {
        const CXCommentInlineCommandRenderKind_Normal = 0,
        const CXCommentInlineCommandRenderKind_Bold = 1,
        const CXCommentInlineCommandRenderKind_Monospaced = 2,
        const CXCommentInlineCommandRenderKind_Emphasized = 3,
    }
}

cenum! {
    enum CXCommentKind {
        const CXComment_Null = 0,
        const CXComment_Text = 1,
        const CXComment_InlineCommand = 2,
        const CXComment_HTMLStartTag = 3,
        const CXComment_HTMLEndTag = 4,
        const CXComment_Paragraph = 5,
        const CXComment_BlockCommand = 6,
        const CXComment_ParamCommand = 7,
        const CXComment_TParamCommand = 8,
        const CXComment_VerbatimBlockCommand = 9,
        const CXComment_VerbatimBlockLine = 10,
        const CXComment_VerbatimLine = 11,
        const CXComment_FullComment = 12,
    }
}

cenum! {
    enum CXCommentParamPassDirection {
        const CXCommentParamPassDirection_In = 0,
        const CXCommentParamPassDirection_Out = 1,
        const CXCommentParamPassDirection_InOut = 2,
    }
}

cenum! {
    enum CXCompilationDatabase_Error {
        const CXCompilationDatabase_NoError = 0,
        const CXCompilationDatabase_CanNotLoadDatabase = 1,
    }
}

cenum! {
    enum CXCompletionChunkKind {
        const CXCompletionChunk_Optional = 0,
        const CXCompletionChunk_TypedText = 1,
        const CXCompletionChunk_Text = 2,
        const CXCompletionChunk_Placeholder = 3,
        const CXCompletionChunk_Informative = 4,
        const CXCompletionChunk_CurrentParameter = 5,
        const CXCompletionChunk_LeftParen = 6,
        const CXCompletionChunk_RightParen = 7,
        const CXCompletionChunk_LeftBracket = 8,
        const CXCompletionChunk_RightBracket = 9,
        const CXCompletionChunk_LeftBrace = 10,
        const CXCompletionChunk_RightBrace = 11,
        const CXCompletionChunk_LeftAngle = 12,
        const CXCompletionChunk_RightAngle = 13,
        const CXCompletionChunk_Comma = 14,
        const CXCompletionChunk_ResultType = 15,
        const CXCompletionChunk_Colon = 16,
        const CXCompletionChunk_SemiColon = 17,
        const CXCompletionChunk_Equal = 18,
        const CXCompletionChunk_HorizontalSpace = 19,
        const CXCompletionChunk_VerticalSpace = 20,
    }
}

cenum! {
    enum CXCursorKind {
        const CXCursor_UnexposedDecl = 1,
        const CXCursor_StructDecl = 2,
        const CXCursor_UnionDecl = 3,
        const CXCursor_ClassDecl = 4,
        const CXCursor_EnumDecl = 5,
        const CXCursor_FieldDecl = 6,
        const CXCursor_EnumConstantDecl = 7,
        const CXCursor_FunctionDecl = 8,
        const CXCursor_VarDecl = 9,
        const CXCursor_ParmDecl = 10,
        const CXCursor_ObjCInterfaceDecl = 11,
        const CXCursor_ObjCCategoryDecl = 12,
        const CXCursor_ObjCProtocolDecl = 13,
        const CXCursor_ObjCPropertyDecl = 14,
        const CXCursor_ObjCIvarDecl = 15,
        const CXCursor_ObjCInstanceMethodDecl = 16,
        const CXCursor_ObjCClassMethodDecl = 17,
        const CXCursor_ObjCImplementationDecl = 18,
        const CXCursor_ObjCCategoryImplDecl = 19,
        const CXCursor_TypedefDecl = 20,
        const CXCursor_CXXMethod = 21,
        const CXCursor_Namespace = 22,
        const CXCursor_LinkageSpec = 23,
        const CXCursor_Constructor = 24,
        const CXCursor_Destructor = 25,
        const CXCursor_ConversionFunction = 26,
        const CXCursor_TemplateTypeParameter = 27,
        const CXCursor_NonTypeTemplateParameter = 28,
        const CXCursor_TemplateTemplateParameter = 29,
        const CXCursor_FunctionTemplate = 30,
        const CXCursor_ClassTemplate = 31,
        const CXCursor_ClassTemplatePartialSpecialization = 32,
        const CXCursor_NamespaceAlias = 33,
        const CXCursor_UsingDirective = 34,
        const CXCursor_UsingDeclaration = 35,
        const CXCursor_TypeAliasDecl = 36,
        const CXCursor_ObjCSynthesizeDecl = 37,
        const CXCursor_ObjCDynamicDecl = 38,
        const CXCursor_CXXAccessSpecifier = 39,
        const CXCursor_ObjCSuperClassRef = 40,
        const CXCursor_ObjCProtocolRef = 41,
        const CXCursor_ObjCClassRef = 42,
        const CXCursor_TypeRef = 43,
        const CXCursor_CXXBaseSpecifier = 44,
        const CXCursor_TemplateRef = 45,
        const CXCursor_NamespaceRef = 46,
        const CXCursor_MemberRef = 47,
        const CXCursor_LabelRef = 48,
        const CXCursor_OverloadedDeclRef = 49,
        const CXCursor_VariableRef = 50,
        const CXCursor_InvalidFile = 70,
        const CXCursor_NoDeclFound = 71,
        const CXCursor_NotImplemented = 72,
        const CXCursor_InvalidCode = 73,
        const CXCursor_UnexposedExpr = 100,
        const CXCursor_DeclRefExpr = 101,
        const CXCursor_MemberRefExpr = 102,
        const CXCursor_CallExpr = 103,
        const CXCursor_ObjCMessageExpr = 104,
        const CXCursor_BlockExpr = 105,
        const CXCursor_IntegerLiteral = 106,
        const CXCursor_FloatingLiteral = 107,
        const CXCursor_ImaginaryLiteral = 108,
        const CXCursor_StringLiteral = 109,
        const CXCursor_CharacterLiteral = 110,
        const CXCursor_ParenExpr = 111,
        const CXCursor_UnaryOperator = 112,
        const CXCursor_ArraySubscriptExpr = 113,
        const CXCursor_BinaryOperator = 114,
        const CXCursor_CompoundAssignOperator = 115,
        const CXCursor_ConditionalOperator = 116,
        const CXCursor_CStyleCastExpr = 117,
        const CXCursor_CompoundLiteralExpr = 118,
        const CXCursor_InitListExpr = 119,
        const CXCursor_AddrLabelExpr = 120,
        const CXCursor_StmtExpr = 121,
        const CXCursor_GenericSelectionExpr = 122,
        const CXCursor_GNUNullExpr = 123,
        const CXCursor_CXXStaticCastExpr = 124,
        const CXCursor_CXXDynamicCastExpr = 125,
        const CXCursor_CXXReinterpretCastExpr = 126,
        const CXCursor_CXXConstCastExpr = 127,
        const CXCursor_CXXFunctionalCastExpr = 128,
        const CXCursor_CXXTypeidExpr = 129,
        const CXCursor_CXXBoolLiteralExpr = 130,
        const CXCursor_CXXNullPtrLiteralExpr = 131,
        const CXCursor_CXXThisExpr = 132,
        const CXCursor_CXXThrowExpr = 133,
        const CXCursor_CXXNewExpr = 134,
        const CXCursor_CXXDeleteExpr = 135,
        const CXCursor_UnaryExpr = 136,
        const CXCursor_ObjCStringLiteral = 137,
        const CXCursor_ObjCEncodeExpr = 138,
        const CXCursor_ObjCSelectorExpr = 139,
        const CXCursor_ObjCProtocolExpr = 140,
        const CXCursor_ObjCBridgedCastExpr = 141,
        const CXCursor_PackExpansionExpr = 142,
        const CXCursor_SizeOfPackExpr = 143,
        const CXCursor_LambdaExpr = 144,
        const CXCursor_ObjCBoolLiteralExpr = 145,
        const CXCursor_ObjCSelfExpr = 146,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_OMPArraySectionExpr = 147,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_ObjCAvailabilityCheckExpr = 148,
        const CXCursor_UnexposedStmt = 200,
        const CXCursor_LabelStmt = 201,
        const CXCursor_CompoundStmt = 202,
        const CXCursor_CaseStmt = 203,
        const CXCursor_DefaultStmt = 204,
        const CXCursor_IfStmt = 205,
        const CXCursor_SwitchStmt = 206,
        const CXCursor_WhileStmt = 207,
        const CXCursor_DoStmt = 208,
        const CXCursor_ForStmt = 209,
        const CXCursor_GotoStmt = 210,
        const CXCursor_IndirectGotoStmt = 211,
        const CXCursor_ContinueStmt = 212,
        const CXCursor_BreakStmt = 213,
        const CXCursor_ReturnStmt = 214,
        /// Duplicate of `CXCursor_GccAsmStmt`.
        const CXCursor_AsmStmt = 215,
        const CXCursor_ObjCAtTryStmt = 216,
        const CXCursor_ObjCAtCatchStmt = 217,
        const CXCursor_ObjCAtFinallyStmt = 218,
        const CXCursor_ObjCAtThrowStmt = 219,
        const CXCursor_ObjCAtSynchronizedStmt = 220,
        const CXCursor_ObjCAutoreleasePoolStmt = 221,
        const CXCursor_ObjCForCollectionStmt = 222,
        const CXCursor_CXXCatchStmt = 223,
        const CXCursor_CXXTryStmt = 224,
        const CXCursor_CXXForRangeStmt = 225,
        const CXCursor_SEHTryStmt = 226,
        const CXCursor_SEHExceptStmt = 227,
        const CXCursor_SEHFinallyStmt = 228,
        const CXCursor_MSAsmStmt = 229,
        const CXCursor_NullStmt = 230,
        const CXCursor_DeclStmt = 231,
        const CXCursor_OMPParallelDirective = 232,
        const CXCursor_OMPSimdDirective = 233,
        const CXCursor_OMPForDirective = 234,
        const CXCursor_OMPSectionsDirective = 235,
        const CXCursor_OMPSectionDirective = 236,
        const CXCursor_OMPSingleDirective = 237,
        const CXCursor_OMPParallelForDirective = 238,
        const CXCursor_OMPParallelSectionsDirective = 239,
        const CXCursor_OMPTaskDirective = 240,
        const CXCursor_OMPMasterDirective = 241,
        const CXCursor_OMPCriticalDirective = 242,
        const CXCursor_OMPTaskyieldDirective = 243,
        const CXCursor_OMPBarrierDirective = 244,
        const CXCursor_OMPTaskwaitDirective = 245,
        const CXCursor_OMPFlushDirective = 246,
        const CXCursor_SEHLeaveStmt = 247,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_OMPOrderedDirective = 248,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_OMPAtomicDirective = 249,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_OMPForSimdDirective = 250,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_OMPParallelForSimdDirective = 251,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_OMPTargetDirective = 252,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_OMPTeamsDirective = 253,
        /// Only produced by `libclang` 3.7 and later.
        const CXCursor_OMPTaskgroupDirective = 254,
        /// Only produced by `libclang` 3.7 and later.
        const CXCursor_OMPCancellationPointDirective = 255,
        /// Only produced by `libclang` 3.7 and later.
        const CXCursor_OMPCancelDirective = 256,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_OMPTargetDataDirective = 257,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_OMPTaskLoopDirective = 258,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_OMPTaskLoopSimdDirective = 259,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_OMPDistributeDirective = 260,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPTargetEnterDataDirective = 261,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPTargetExitDataDirective = 262,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPTargetParallelDirective = 263,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPTargetParallelForDirective = 264,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPTargetUpdateDirective = 265,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPDistributeParallelForDirective = 266,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPDistributeParallelForSimdDirective = 267,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPDistributeSimdDirective = 268,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_OMPTargetParallelForSimdDirective = 269,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTargetSimdDirective = 270,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTeamsDistributeDirective = 271,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTeamsDistributeSimdDirective = 272,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTeamsDistributeParallelForSimdDirective = 273,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTeamsDistributeParallelForDirective = 274,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTargetTeamsDirective = 275,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTargetTeamsDistributeDirective = 276,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTargetTeamsDistributeParallelForDirective = 277,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_OMPTargetTeamsDistributeParallelForSimdDirective = 278,
        /// Only producer by `libclang` 4.0 and later.
        const CXCursor_OMPTargetTeamsDistributeSimdDirective = 279,
        const CXCursor_TranslationUnit = 300,
        const CXCursor_UnexposedAttr = 400,
        const CXCursor_IBActionAttr = 401,
        const CXCursor_IBOutletAttr = 402,
        const CXCursor_IBOutletCollectionAttr = 403,
        const CXCursor_CXXFinalAttr = 404,
        const CXCursor_CXXOverrideAttr = 405,
        const CXCursor_AnnotateAttr = 406,
        const CXCursor_AsmLabelAttr = 407,
        const CXCursor_PackedAttr = 408,
        const CXCursor_PureAttr = 409,
        const CXCursor_ConstAttr = 410,
        const CXCursor_NoDuplicateAttr = 411,
        const CXCursor_CUDAConstantAttr = 412,
        const CXCursor_CUDADeviceAttr = 413,
        const CXCursor_CUDAGlobalAttr = 414,
        const CXCursor_CUDAHostAttr = 415,
        /// Only produced by `libclang` 3.6 and later.
        const CXCursor_CUDASharedAttr = 416,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_VisibilityAttr = 417,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_DLLExport = 418,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_DLLImport = 419,
        const CXCursor_PreprocessingDirective = 500,
        const CXCursor_MacroDefinition = 501,
        /// Duplicate of `CXCursor_MacroInstantiation`.
        const CXCursor_MacroExpansion = 502,
        const CXCursor_InclusionDirective = 503,
        const CXCursor_ModuleImportDecl = 600,
        /// Only produced by `libclang` 3.8 and later.
        const CXCursor_TypeAliasTemplateDecl = 601,
        /// Only produced by `libclang` 3.9 and later.
        const CXCursor_StaticAssert = 602,
        /// Only produced by `libclang` 4.0 and later.
        const CXCursor_FriendDecl = 603,
        /// Only produced by `libclang` 3.7 and later.
        const CXCursor_OverloadCandidate = 700,
    }
}

cenum! {
    #[cfg(feature="gte_clang_5_0")]
    enum CXCursor_ExceptionSpecificationKind {
        const CXCursor_ExceptionSpecificationKind_None = 0,
        const CXCursor_ExceptionSpecificationKind_DynamicNone = 1,
        const CXCursor_ExceptionSpecificationKind_Dynamic = 2,
        const CXCursor_ExceptionSpecificationKind_MSAny = 3,
        const CXCursor_ExceptionSpecificationKind_BasicNoexcept = 4,
        const CXCursor_ExceptionSpecificationKind_ComputedNoexcept = 5,
        const CXCursor_ExceptionSpecificationKind_Unevaluated = 6,
        const CXCursor_ExceptionSpecificationKind_Uninstantiated = 7,
        const CXCursor_ExceptionSpecificationKind_Unparsed = 8,
    }
}

cenum! {
    enum CXDiagnosticSeverity {
        const CXDiagnostic_Ignored = 0,
        const CXDiagnostic_Note = 1,
        const CXDiagnostic_Warning = 2,
        const CXDiagnostic_Error = 3,
        const CXDiagnostic_Fatal = 4,
    }
}

cenum! {
    enum CXErrorCode {
        const CXError_Success = 0,
        const CXError_Failure = 1,
        const CXError_Crashed = 2,
        const CXError_InvalidArguments = 3,
        const CXError_ASTReadError = 4,
    }
}

cenum! {
    enum CXEvalResultKind {
        const CXEval_UnExposed = 0,
        const CXEval_Int = 1 ,
        const CXEval_Float = 2,
        const CXEval_ObjCStrLiteral = 3,
        const CXEval_StrLiteral = 4,
        const CXEval_CFStr = 5,
        const CXEval_Other = 6,
    }
}

cenum! {
    enum CXIdxAttrKind {
        const CXIdxAttr_Unexposed = 0,
        const CXIdxAttr_IBAction = 1,
        const CXIdxAttr_IBOutlet = 2,
        const CXIdxAttr_IBOutletCollection = 3,
    }
}

cenum! {
    enum CXIdxEntityCXXTemplateKind {
        const CXIdxEntity_NonTemplate = 0,
        const CXIdxEntity_Template = 1,
        const CXIdxEntity_TemplatePartialSpecialization = 2,
        const CXIdxEntity_TemplateSpecialization = 3,
    }
}

cenum! {
    enum CXIdxEntityKind {
        const CXIdxEntity_Unexposed = 0,
        const CXIdxEntity_Typedef = 1,
        const CXIdxEntity_Function = 2,
        const CXIdxEntity_Variable = 3,
        const CXIdxEntity_Field = 4,
        const CXIdxEntity_EnumConstant = 5,
        const CXIdxEntity_ObjCClass = 6,
        const CXIdxEntity_ObjCProtocol = 7,
        const CXIdxEntity_ObjCCategory = 8,
        const CXIdxEntity_ObjCInstanceMethod = 9,
        const CXIdxEntity_ObjCClassMethod = 10,
        const CXIdxEntity_ObjCProperty = 11,
        const CXIdxEntity_ObjCIvar = 12,
        const CXIdxEntity_Enum = 13,
        const CXIdxEntity_Struct = 14,
        const CXIdxEntity_Union = 15,
        const CXIdxEntity_CXXClass = 16,
        const CXIdxEntity_CXXNamespace = 17,
        const CXIdxEntity_CXXNamespaceAlias = 18,
        const CXIdxEntity_CXXStaticVariable = 19,
        const CXIdxEntity_CXXStaticMethod = 20,
        const CXIdxEntity_CXXInstanceMethod = 21,
        const CXIdxEntity_CXXConstructor = 22,
        const CXIdxEntity_CXXDestructor = 23,
        const CXIdxEntity_CXXConversionFunction = 24,
        const CXIdxEntity_CXXTypeAlias = 25,
        const CXIdxEntity_CXXInterface = 26,
    }
}

cenum! {
    enum CXIdxEntityLanguage {
        const CXIdxEntityLang_None = 0,
        const CXIdxEntityLang_C = 1,
        const CXIdxEntityLang_ObjC = 2,
        const CXIdxEntityLang_CXX = 3,
        /// Only produced by `libclang` 5.0 and later.
        const CXIdxEntityLang_Swift = 4,
    }
}

cenum! {
    enum CXIdxEntityRefKind {
        const CXIdxEntityRef_Direct = 1,
        const CXIdxEntityRef_Implicit = 2,
    }
}

cenum! {
    enum CXIdxObjCContainerKind {
        const CXIdxObjCContainer_ForwardRef = 0,
        const CXIdxObjCContainer_Interface = 1,
        const CXIdxObjCContainer_Implementation = 2,
    }
}

cenum! {
    enum CXLanguageKind {
        const CXLanguage_Invalid = 0,
        const CXLanguage_C = 1,
        const CXLanguage_ObjC = 2,
        const CXLanguage_CPlusPlus = 3,
    }
}

cenum! {
    enum CXLinkageKind {
        const CXLinkage_Invalid = 0,
        const CXLinkage_NoLinkage = 1,
        const CXLinkage_Internal = 2,
        const CXLinkage_UniqueExternal = 3,
        const CXLinkage_External = 4,
    }
}

cenum! {
    enum CXLoadDiag_Error {
        const CXLoadDiag_None = 0,
        const CXLoadDiag_Unknown = 1,
        const CXLoadDiag_CannotLoad = 2,
        const CXLoadDiag_InvalidFile = 3,
    }
}

cenum! {
    enum CXRefQualifierKind {
        const CXRefQualifier_None = 0,
        const CXRefQualifier_LValue = 1,
        const CXRefQualifier_RValue = 2,
    }
}

cenum! {
    enum CXResult {
        const CXResult_Success = 0,
        const CXResult_Invalid = 1,
        const CXResult_VisitBreak = 2,
    }
}

cenum! {
    enum CXSaveError {
        const CXSaveError_None = 0,
        const CXSaveError_Unknown = 1,
        const CXSaveError_TranslationErrors = 2,
        const CXSaveError_InvalidTU = 3,
    }
}

cenum! {
    enum CXTUResourceUsageKind {
        const CXTUResourceUsage_AST = 1,
        const CXTUResourceUsage_Identifiers = 2,
        const CXTUResourceUsage_Selectors = 3,
        const CXTUResourceUsage_GlobalCompletionResults = 4,
        const CXTUResourceUsage_SourceManagerContentCache = 5,
        const CXTUResourceUsage_AST_SideTables = 6,
        const CXTUResourceUsage_SourceManager_Membuffer_Malloc = 7,
        const CXTUResourceUsage_SourceManager_Membuffer_MMap = 8,
        const CXTUResourceUsage_ExternalASTSource_Membuffer_Malloc = 9,
        const CXTUResourceUsage_ExternalASTSource_Membuffer_MMap = 10,
        const CXTUResourceUsage_Preprocessor = 11,
        const CXTUResourceUsage_PreprocessingRecord = 12,
        const CXTUResourceUsage_SourceManager_DataStructures = 13,
        const CXTUResourceUsage_Preprocessor_HeaderSearch = 14,
    }
}

cenum! {
    #[cfg(feature="gte_clang_3_6")]
    enum CXTemplateArgumentKind {
        const CXTemplateArgumentKind_Null = 0,
        const CXTemplateArgumentKind_Type = 1,
        const CXTemplateArgumentKind_Declaration = 2,
        const CXTemplateArgumentKind_NullPtr = 3,
        const CXTemplateArgumentKind_Integral = 4,
        const CXTemplateArgumentKind_Template = 5,
        const CXTemplateArgumentKind_TemplateExpansion = 6,
        const CXTemplateArgumentKind_Expression = 7,
        const CXTemplateArgumentKind_Pack = 8,
        const CXTemplateArgumentKind_Invalid = 9,
    }
}

cenum! {
    enum CXTokenKind {
        const CXToken_Punctuation = 0,
        const CXToken_Keyword = 1,
        const CXToken_Identifier = 2,
        const CXToken_Literal = 3,
        const CXToken_Comment = 4,
    }
}

cenum! {
    enum CXTypeKind {
        const CXType_Invalid = 0,
        const CXType_Unexposed = 1,
        const CXType_Void = 2,
        const CXType_Bool = 3,
        const CXType_Char_U = 4,
        const CXType_UChar = 5,
        const CXType_Char16 = 6,
        const CXType_Char32 = 7,
        const CXType_UShort = 8,
        const CXType_UInt = 9,
        const CXType_ULong = 10,
        const CXType_ULongLong = 11,
        const CXType_UInt128 = 12,
        const CXType_Char_S = 13,
        const CXType_SChar = 14,
        const CXType_WChar = 15,
        const CXType_Short = 16,
        const CXType_Int = 17,
        const CXType_Long = 18,
        const CXType_LongLong = 19,
        const CXType_Int128 = 20,
        const CXType_Float = 21,
        const CXType_Double = 22,
        const CXType_LongDouble = 23,
        const CXType_NullPtr = 24,
        const CXType_Overload = 25,
        const CXType_Dependent = 26,
        const CXType_ObjCId = 27,
        const CXType_ObjCClass = 28,
        const CXType_ObjCSel = 29,
        /// Only produced by `libclang` 3.9 and later.
        const CXType_Float128 = 30,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_Half = 31,
        const CXType_Complex = 100,
        const CXType_Pointer = 101,
        const CXType_BlockPointer = 102,
        const CXType_LValueReference = 103,
        const CXType_RValueReference = 104,
        const CXType_Record = 105,
        const CXType_Enum = 106,
        const CXType_Typedef = 107,
        const CXType_ObjCInterface = 108,
        const CXType_ObjCObjectPointer = 109,
        const CXType_FunctionNoProto = 110,
        const CXType_FunctionProto = 111,
        const CXType_ConstantArray = 112,
        const CXType_Vector = 113,
        const CXType_IncompleteArray = 114,
        const CXType_VariableArray = 115,
        const CXType_DependentSizedArray = 116,
        const CXType_MemberPointer = 117,
        /// Only produced by `libclang` 3.8 and later.
        const CXType_Auto = 118,
        /// Only produced by `libclang` 3.9 and later.
        const CXType_Elaborated = 119,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_Pipe = 120,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dRO = 121,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dArrayRO = 122,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dBufferRO = 123,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dRO = 124,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayRO = 125,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dDepthRO = 126,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayDepthRO = 127,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dMSAARO = 128,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayMSAARO = 129,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dMSAADepthRO = 130,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayMSAADepthRO = 131,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage3dRO = 132,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dWO = 133,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dArrayWO = 134,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dBufferWO = 135,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dWO = 136,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayWO = 137,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dDepthWO = 138,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayDepthWO = 139,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dMSAAWO = 140,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayMSAAWO = 141,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dMSAADepthWO = 142,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayMSAADepthWO = 143,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage3dWO = 144,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dRW = 145,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dArrayRW = 146,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage1dBufferRW = 147,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dRW = 148,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayRW = 149,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dDepthRW = 150,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayDepthRW = 151,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dMSAARW = 152,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayMSAARW = 153,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dMSAADepthRW = 154,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage2dArrayMSAADepthRW = 155,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLImage3dRW = 156,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLSampler = 157,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLEvent = 158,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLQueue = 159,
        /// Only produced by `libclang` 5.0 and later.
        const CXType_OCLReserveID = 160,
    }
}

cenum! {
    enum CXTypeLayoutError {
        const CXTypeLayoutError_Invalid = -1,
        const CXTypeLayoutError_Incomplete = -2,
        const CXTypeLayoutError_Dependent = -3,
        const CXTypeLayoutError_NotConstantSize = -4,
        const CXTypeLayoutError_InvalidFieldName = -5,
    }
}

cenum! {
    #[cfg(feature="gte_clang_3_8")]
    enum CXVisibilityKind {
        const CXVisibility_Invalid = 0,
        const CXVisibility_Hidden = 1,
        const CXVisibility_Protected = 2,
        const CXVisibility_Default = 3,
    }
}

cenum! {
    enum CXVisitorResult {
        const CXVisit_Break = 0,
        const CXVisit_Continue = 1,
    }
}

cenum! {
    enum CX_CXXAccessSpecifier {
        const CX_CXXInvalidAccessSpecifier = 0,
        const CX_CXXPublic = 1,
        const CX_CXXProtected = 2,
        const CX_CXXPrivate = 3,
    }
}

cenum! {
    #[cfg(feature="gte_clang_3_6")]
    enum CX_StorageClass {
        const CX_SC_Invalid = 0,
        const CX_SC_None = 1,
        const CX_SC_Extern = 2,
        const CX_SC_Static = 3,
        const CX_SC_PrivateExtern = 4,
        const CX_SC_OpenCLWorkGroupLocal = 5,
        const CX_SC_Auto = 6,
        const CX_SC_Register = 7,
    }
}

//================================================
// Flags
//================================================

cenum! {
    enum CXCodeComplete_Flags {
        const CXCodeComplete_IncludeMacros = 1;
        const CXCodeComplete_IncludeCodePatterns = 2;
        const CXCodeComplete_IncludeBriefComments = 4;
    }
}

cenum! {
    enum CXCompletionContext {
        const CXCompletionContext_Unexposed = 0;
        const CXCompletionContext_AnyType = 1;
        const CXCompletionContext_AnyValue = 2;
        const CXCompletionContext_ObjCObjectValue = 4;
        const CXCompletionContext_ObjCSelectorValue = 8;
        const CXCompletionContext_CXXClassTypeValue = 16;
        const CXCompletionContext_DotMemberAccess = 32;
        const CXCompletionContext_ArrowMemberAccess = 64;
        const CXCompletionContext_ObjCPropertyAccess = 128;
        const CXCompletionContext_EnumTag = 256;
        const CXCompletionContext_UnionTag = 512;
        const CXCompletionContext_StructTag = 1024;
        const CXCompletionContext_ClassTag = 2048;
        const CXCompletionContext_Namespace = 4096;
        const CXCompletionContext_NestedNameSpecifier = 8192;
        const CXCompletionContext_ObjCInterface = 16384;
        const CXCompletionContext_ObjCProtocol = 32768;
        const CXCompletionContext_ObjCCategory = 65536;
        const CXCompletionContext_ObjCInstanceMessage = 131072;
        const CXCompletionContext_ObjCClassMessage = 262144;
        const CXCompletionContext_ObjCSelectorName = 524288;
        const CXCompletionContext_MacroName = 1048576;
        const CXCompletionContext_NaturalLanguage = 2097152;
        const CXCompletionContext_Unknown = 4194303;
    }
}

cenum! {
    enum CXDiagnosticDisplayOptions {
        const CXDiagnostic_DisplaySourceLocation = 1;
        const CXDiagnostic_DisplayColumn = 2;
        const CXDiagnostic_DisplaySourceRanges = 4;
        const CXDiagnostic_DisplayOption = 8;
        const CXDiagnostic_DisplayCategoryId = 16;
        const CXDiagnostic_DisplayCategoryName = 32;
    }
}

cenum! {
    enum CXGlobalOptFlags {
        const CXGlobalOpt_None = 0;
        const CXGlobalOpt_ThreadBackgroundPriorityForIndexing = 1;
        const CXGlobalOpt_ThreadBackgroundPriorityForEditing = 2;
        const CXGlobalOpt_ThreadBackgroundPriorityForAll = 3;
    }
}

cenum! {
    enum CXIdxDeclInfoFlags {
        const CXIdxDeclFlag_Skipped = 1;
    }
}

cenum! {
    enum CXIndexOptFlags {
        const CXIndexOptNone = 0;
        const CXIndexOptSuppressRedundantRefs = 1;
        const CXIndexOptIndexFunctionLocalSymbols = 2;
        const CXIndexOptIndexImplicitTemplateInstantiations = 4;
        const CXIndexOptSuppressWarnings = 8;
        const CXIndexOptSkipParsedBodiesInSession = 16;
    }
}

cenum! {
    enum CXNameRefFlags {
        const CXNameRange_WantQualifier = 1;
        const CXNameRange_WantTemplateArgs = 2;
        const CXNameRange_WantSinglePiece = 4;
    }
}

cenum! {
    enum CXObjCDeclQualifierKind {
        const CXObjCDeclQualifier_None = 0;
        const CXObjCDeclQualifier_In = 1;
        const CXObjCDeclQualifier_Inout = 2;
        const CXObjCDeclQualifier_Out = 4;
        const CXObjCDeclQualifier_Bycopy = 8;
        const CXObjCDeclQualifier_Byref = 16;
        const CXObjCDeclQualifier_Oneway = 32;
    }
}

cenum! {
    enum CXObjCPropertyAttrKind {
        const CXObjCPropertyAttr_noattr = 0;
        const CXObjCPropertyAttr_readonly = 1;
        const CXObjCPropertyAttr_getter = 2;
        const CXObjCPropertyAttr_assign = 4;
        const CXObjCPropertyAttr_readwrite = 8;
        const CXObjCPropertyAttr_retain = 16;
        const CXObjCPropertyAttr_copy = 32;
        const CXObjCPropertyAttr_nonatomic = 64;
        const CXObjCPropertyAttr_setter = 128;
        const CXObjCPropertyAttr_atomic = 256;
        const CXObjCPropertyAttr_weak = 512;
        const CXObjCPropertyAttr_strong = 1024;
        const CXObjCPropertyAttr_unsafe_unretained = 2048;
        #[cfg(feature="gte_clang_3_9")]
        const CXObjCPropertyAttr_class = 4096;
    }
}

cenum! {
    enum CXReparse_Flags {
        const CXReparse_None = 0;
    }
}

cenum! {
    enum CXSaveTranslationUnit_Flags {
        const CXSaveTranslationUnit_None = 0;
    }
}

cenum! {
    enum CXTranslationUnit_Flags {
        const CXTranslationUnit_None = 0;
        const CXTranslationUnit_DetailedPreprocessingRecord = 1;
        const CXTranslationUnit_Incomplete = 2;
        const CXTranslationUnit_PrecompiledPreamble = 4;
        const CXTranslationUnit_CacheCompletionResults = 8;
        const CXTranslationUnit_ForSerialization = 16;
        const CXTranslationUnit_CXXChainedPCH = 32;
        const CXTranslationUnit_SkipFunctionBodies = 64;
        const CXTranslationUnit_IncludeBriefCommentsInCodeCompletion = 128;
        #[cfg(feature="gte_clang_3_8")]
        const CXTranslationUnit_CreatePreambleOnFirstParse = 256;
        #[cfg(feature="gte_clang_3_9")]
        const CXTranslationUnit_KeepGoing = 512;
        #[cfg(feature="gte_clang_5_0")]
        const CXTranslationUnit_SingleFileParse = 1024;
    }
}

//================================================
// Structs
//================================================

// Opaque ________________________________________

macro_rules! opaque { ($name:ident) => (pub type $name = *mut c_void;); }

opaque!(CXCompilationDatabase);
opaque!(CXCompileCommand);
opaque!(CXCompileCommands);
opaque!(CXCompletionString);
opaque!(CXCursorSet);
opaque!(CXDiagnostic);
opaque!(CXDiagnosticSet);
#[cfg(feature="gte_clang_3_9")]
opaque!(CXEvalResult);
opaque!(CXFile);
opaque!(CXIdxClientASTFile);
opaque!(CXIdxClientContainer);
opaque!(CXIdxClientEntity);
opaque!(CXIdxClientFile);
opaque!(CXIndex);
opaque!(CXIndexAction);
opaque!(CXModule);
opaque!(CXRemapping);
#[cfg(feature="gte_clang_5_0")]
opaque!(CXTargetInfo);
opaque!(CXTranslationUnit);

// Transparent ___________________________________

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXCodeCompleteResults {
    pub Results: *mut CXCompletionResult,
    pub NumResults: c_uint,
}

default!(CXCodeCompleteResults);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXComment {
    pub ASTNode: *const c_void,
    pub TranslationUnit: CXTranslationUnit,
}

default!(CXComment);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXCompletionResult {
    pub CursorKind: CXCursorKind,
    pub CompletionString: CXCompletionString,
}

default!(CXCompletionResult);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXCursor {
    pub kind: CXCursorKind,
    pub xdata: c_int,
    pub data: [*const c_void; 3],
}

default!(CXCursor);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXCursorAndRangeVisitor {
    pub context: *mut c_void,
    pub visit: extern fn(*mut c_void, CXCursor, CXSourceRange) -> CXVisitorResult,
}

default!(CXCursorAndRangeVisitor);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXFileUniqueID {
    pub data: [c_ulonglong; 3],
}

default!(CXFileUniqueID);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxAttrInfo {
    pub kind: CXIdxAttrKind,
    pub cursor: CXCursor,
    pub loc: CXIdxLoc,
}

default!(CXIdxAttrInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxBaseClassInfo {
    pub base: *const CXIdxEntityInfo,
    pub cursor: CXCursor,
    pub loc: CXIdxLoc,
}

default!(CXIdxBaseClassInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxCXXClassDeclInfo {
    pub declInfo: *const CXIdxDeclInfo,
    pub bases: *const *const CXIdxBaseClassInfo,
    pub numBases: c_uint,
}

default!(CXIdxCXXClassDeclInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxContainerInfo {
    pub cursor: CXCursor,
}

default!(CXIdxContainerInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxDeclInfo {
    pub entityInfo: *const CXIdxEntityInfo,
    pub cursor: CXCursor,
    pub loc: CXIdxLoc,
    pub semanticContainer: *const CXIdxContainerInfo,
    pub lexicalContainer: *const CXIdxContainerInfo,
    pub isRedeclaration: c_int,
    pub isDefinition: c_int,
    pub isContainer: c_int,
    pub declAsContainer: *const CXIdxContainerInfo,
    pub isImplicit: c_int,
    pub attributes: *const *const CXIdxAttrInfo,
    pub numAttributes: c_uint,
    pub flags: c_uint,
}

default!(CXIdxDeclInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxEntityInfo {
    pub kind: CXIdxEntityKind,
    pub templateKind: CXIdxEntityCXXTemplateKind,
    pub lang: CXIdxEntityLanguage,
    pub name: *const c_char,
    pub USR: *const c_char,
    pub cursor: CXCursor,
    pub attributes: *const *const CXIdxAttrInfo,
    pub numAttributes: c_uint,
}

default!(CXIdxEntityInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxEntityRefInfo {
    pub kind: CXIdxEntityRefKind,
    pub cursor: CXCursor,
    pub loc: CXIdxLoc,
    pub referencedEntity: *const CXIdxEntityInfo,
    pub parentEntity: *const CXIdxEntityInfo,
    pub container: *const CXIdxContainerInfo,
}

default!(CXIdxEntityRefInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxIBOutletCollectionAttrInfo {
    pub attrInfo: *const CXIdxAttrInfo,
    pub objcClass: *const CXIdxEntityInfo,
    pub classCursor: CXCursor,
    pub classLoc: CXIdxLoc,
}

default!(CXIdxIBOutletCollectionAttrInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxImportedASTFileInfo {
    pub file: CXFile,
    pub module: CXModule,
    pub loc: CXIdxLoc,
    pub isImplicit: c_int,
}

default!(CXIdxImportedASTFileInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxIncludedFileInfo {
    pub hashLoc: CXIdxLoc,
    pub filename: *const c_char,
    pub file: CXFile,
    pub isImport: c_int,
    pub isAngled: c_int,
    pub isModuleImport: c_int,
}

default!(CXIdxIncludedFileInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxLoc {
    pub ptr_data: [*mut c_void; 2],
    pub int_data: c_uint,
}

default!(CXIdxLoc);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxObjCCategoryDeclInfo {
    pub containerInfo: *const CXIdxObjCContainerDeclInfo,
    pub objcClass: *const CXIdxEntityInfo,
    pub classCursor: CXCursor,
    pub classLoc: CXIdxLoc,
    pub protocols: *const CXIdxObjCProtocolRefListInfo,
}

default!(CXIdxObjCCategoryDeclInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxObjCContainerDeclInfo {
    pub declInfo: *const CXIdxDeclInfo,
    pub kind: CXIdxObjCContainerKind,
}

default!(CXIdxObjCContainerDeclInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxObjCInterfaceDeclInfo {
    pub containerInfo: *const CXIdxObjCContainerDeclInfo,
    pub superInfo: *const CXIdxBaseClassInfo,
    pub protocols: *const CXIdxObjCProtocolRefListInfo,
}

default!(CXIdxObjCInterfaceDeclInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxObjCPropertyDeclInfo {
    pub declInfo: *const CXIdxDeclInfo,
    pub getter: *const CXIdxEntityInfo,
    pub setter: *const CXIdxEntityInfo,
}

default!(CXIdxObjCPropertyDeclInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxObjCProtocolRefInfo {
    pub protocol: *const CXIdxEntityInfo,
    pub cursor: CXCursor,
    pub loc: CXIdxLoc,
}

default!(CXIdxObjCProtocolRefInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXIdxObjCProtocolRefListInfo {
    pub protocols: *const *const CXIdxObjCProtocolRefInfo,
    pub numProtocols: c_uint,
}

default!(CXIdxObjCProtocolRefListInfo);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXPlatformAvailability {
    pub Platform: CXString,
    pub Introduced: CXVersion,
    pub Deprecated: CXVersion,
    pub Obsoleted: CXVersion,
    pub Unavailable: c_int,
    pub Message: CXString,
}

default!(CXPlatformAvailability);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXSourceLocation {
    pub ptr_data: [*const c_void; 2],
    pub int_data: c_uint,
}

default!(CXSourceLocation);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXSourceRange {
    pub ptr_data: [*const c_void; 2],
    pub begin_int_data: c_uint,
    pub end_int_data: c_uint,
}

default!(CXSourceRange);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXSourceRangeList {
    pub count: c_uint,
    pub ranges: *mut CXSourceRange,
}

default!(CXSourceRangeList);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXString {
    pub data: *const c_void,
    pub private_flags: c_uint,
}

default!(CXString);

#[cfg(feature="gte_clang_3_8")]
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXStringSet {
    pub Strings: *mut CXString,
    pub Count: c_uint,
}

default!(#[cfg(feature="gte_clang_3_8")] CXStringSet);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXTUResourceUsage {
    pub data: *mut c_void,
    pub numEntries: c_uint,
    pub entries: *mut CXTUResourceUsageEntry,
}

default!(CXTUResourceUsage);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXTUResourceUsageEntry {
    pub kind: CXTUResourceUsageKind,
    pub amount: c_ulong,
}

default!(CXTUResourceUsageEntry);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXToken {
    pub int_data: [c_uint; 4],
    pub ptr_data: *mut c_void,
}

default!(CXToken);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXType {
    pub kind: CXTypeKind,
    pub data: [*mut c_void; 2],
}

default!(CXType);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXUnsavedFile {
    pub Filename: *const c_char,
    pub Contents: *const c_char,
    pub Length: c_ulong,
}

default!(CXUnsavedFile);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct CXVersion {
    pub Major: c_int,
    pub Minor: c_int,
    pub Subminor: c_int,
}

default!(CXVersion);

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct IndexerCallbacks {
    pub abortQuery: extern fn(CXClientData, *mut c_void) -> c_int,
    pub diagnostic: extern fn(CXClientData, CXDiagnosticSet, *mut c_void),
    pub enteredMainFile: extern fn(CXClientData, CXFile, *mut c_void) -> CXIdxClientFile,
    pub ppIncludedFile: extern fn(CXClientData, *const CXIdxIncludedFileInfo) -> CXIdxClientFile,
    pub importedASTFile: extern fn(CXClientData, *const CXIdxImportedASTFileInfo) -> CXIdxClientASTFile,
    pub startedTranslationUnit: extern fn(CXClientData, *mut c_void) -> CXIdxClientContainer,
    pub indexDeclaration: extern fn(CXClientData, *const CXIdxDeclInfo),
    pub indexEntityReference: extern fn(CXClientData, *const CXIdxEntityRefInfo),
}

default!(IndexerCallbacks);

//================================================
// Functions
//================================================

link! {
    pub fn clang_CXCursorSet_contains(set: CXCursorSet, cursor: CXCursor) -> c_uint;
    pub fn clang_CXCursorSet_insert(set: CXCursorSet, cursor: CXCursor) -> c_uint;
    pub fn clang_CXIndex_getGlobalOptions(index: CXIndex) -> CXGlobalOptFlags;
    pub fn clang_CXIndex_setGlobalOptions(index: CXIndex, flags: CXGlobalOptFlags);
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_CXXConstructor_isConvertingConstructor(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_CXXConstructor_isCopyConstructor(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_CXXConstructor_isDefaultConstructor(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_CXXConstructor_isMoveConstructor(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_CXXField_isMutable(cursor: CXCursor) -> c_uint;
    pub fn clang_CXXMethod_isConst(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_CXXMethod_isDefaulted(cursor: CXCursor) -> c_uint;
    pub fn clang_CXXMethod_isPureVirtual(cursor: CXCursor) -> c_uint;
    pub fn clang_CXXMethod_isStatic(cursor: CXCursor) -> c_uint;
    pub fn clang_CXXMethod_isVirtual(cursor: CXCursor) -> c_uint;
    pub fn clang_CompilationDatabase_dispose(database: CXCompilationDatabase);
    pub fn clang_CompilationDatabase_fromDirectory(directory: *const c_char, error: *mut CXCompilationDatabase_Error) -> CXCompilationDatabase;
    pub fn clang_CompilationDatabase_getAllCompileCommands(database: CXCompilationDatabase) -> CXCompileCommands;
    pub fn clang_CompilationDatabase_getCompileCommands(database: CXCompilationDatabase, filename: *const c_char) -> CXCompileCommands;
    pub fn clang_CompileCommand_getArg(command: CXCompileCommand, index: c_uint) -> CXString;
    pub fn clang_CompileCommand_getDirectory(command: CXCompileCommand) -> CXString;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_CompileCommand_getFilename(command: CXCompileCommand) -> CXString;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_CompileCommand_getMappedSourceContent(command: CXCompileCommand, index: c_uint) -> CXString;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_CompileCommand_getMappedSourcePath(command: CXCompileCommand, index: c_uint) -> CXString;
    pub fn clang_CompileCommand_getNumArgs(command: CXCompileCommand) -> c_uint;
    pub fn clang_CompileCommands_dispose(command: CXCompileCommands);
    pub fn clang_CompileCommands_getCommand(command: CXCompileCommands, index: c_uint) -> CXCompileCommand;
    pub fn clang_CompileCommands_getSize(command: CXCompileCommands) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Cursor_Evaluate(cursor: CXCursor) -> CXEvalResult;
    pub fn clang_Cursor_getArgument(cursor: CXCursor, index: c_uint) -> CXCursor;
    pub fn clang_Cursor_getBriefCommentText(cursor: CXCursor) -> CXString;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_Cursor_getCXXManglings(cursor: CXCursor) -> *mut CXStringSet;
    pub fn clang_Cursor_getCommentRange(cursor: CXCursor) -> CXSourceRange;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getMangling(cursor: CXCursor) -> CXString;
    pub fn clang_Cursor_getModule(cursor: CXCursor) -> CXModule;
    pub fn clang_Cursor_getNumArguments(cursor: CXCursor) -> c_int;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getNumTemplateArguments(cursor: CXCursor) -> c_int;
    pub fn clang_Cursor_getObjCDeclQualifiers(cursor: CXCursor) -> CXObjCDeclQualifierKind;
    pub fn clang_Cursor_getObjCPropertyAttributes(cursor: CXCursor, reserved: c_uint) -> CXObjCPropertyAttrKind;
    pub fn clang_Cursor_getObjCSelectorIndex(cursor: CXCursor) -> c_int;
    #[cfg(feature="gte_clang_3_7")]
    pub fn clang_Cursor_getOffsetOfField(cursor: CXCursor) -> c_longlong;
    pub fn clang_Cursor_getRawCommentText(cursor: CXCursor) -> CXString;
    pub fn clang_Cursor_getReceiverType(cursor: CXCursor) -> CXType;
    pub fn clang_Cursor_getSpellingNameRange(cursor: CXCursor, index: c_uint, reserved: c_uint) -> CXSourceRange;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getStorageClass(cursor: CXCursor) -> CX_StorageClass;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getTemplateArgumentKind(cursor: CXCursor, index: c_uint) -> CXTemplateArgumentKind;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getTemplateArgumentType(cursor: CXCursor, index: c_uint) -> CXType;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getTemplateArgumentUnsignedValue(cursor: CXCursor, index: c_uint) -> c_ulonglong;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_Cursor_getTemplateArgumentValue(cursor: CXCursor, index: c_uint) -> c_longlong;
    pub fn clang_Cursor_getTranslationUnit(cursor: CXCursor) -> CXTranslationUnit;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Cursor_hasAttrs(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_7")]
    pub fn clang_Cursor_isAnonymous(cursor: CXCursor) -> c_uint;
    pub fn clang_Cursor_isBitField(cursor: CXCursor) -> c_uint;
    pub fn clang_Cursor_isDynamicCall(cursor: CXCursor) -> c_int;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_Cursor_isExternalSymbol(cursor: CXCursor, language: *mut CXString, from: *mut CXString, generated: *mut c_uint) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Cursor_isFunctionInlined(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Cursor_isMacroBuiltin(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Cursor_isMacroFunctionLike(cursor: CXCursor) -> c_uint;
    pub fn clang_Cursor_isNull(cursor: CXCursor) -> c_int;
    pub fn clang_Cursor_isObjCOptional(cursor: CXCursor) -> c_uint;
    pub fn clang_Cursor_isVariadic(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_EnumDecl_isScoped(cursor: CXCursor) -> c_uint;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_EvalResult_dispose(result: CXEvalResult);
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_EvalResult_getAsDouble(result: CXEvalResult) -> libc::c_double;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_EvalResult_getAsInt(result: CXEvalResult) -> c_int;
    #[cfg(feature="gte_clang_4_0")]
    pub fn clang_EvalResult_getAsLongLong(result: CXEvalResult) -> c_longlong;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_EvalResult_getAsStr(result: CXEvalResult) -> *const c_char;
    #[cfg(feature="gte_clang_4_0")]
    pub fn clang_EvalResult_getAsUnsigned(result: CXEvalResult) -> c_ulonglong;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_EvalResult_getKind(result: CXEvalResult) -> CXEvalResultKind;
    #[cfg(feature="gte_clang_4_0")]
    pub fn clang_EvalResult_isUnsignedInt(result: CXEvalResult) -> c_uint;
    #[cfg(feature="gte_clang_3_6")]
    pub fn clang_File_isEqual(left: CXFile, right: CXFile) -> c_int;
    pub fn clang_IndexAction_create(index: CXIndex) -> CXIndexAction;
    pub fn clang_IndexAction_dispose(index: CXIndexAction);
    pub fn clang_Location_isFromMainFile(location: CXSourceLocation) -> c_int;
    pub fn clang_Location_isInSystemHeader(location: CXSourceLocation) -> c_int;
    pub fn clang_Module_getASTFile(module: CXModule) -> CXFile;
    pub fn clang_Module_getFullName(module: CXModule) -> CXString;
    pub fn clang_Module_getName(module: CXModule) -> CXString;
    pub fn clang_Module_getNumTopLevelHeaders(tu: CXTranslationUnit, module: CXModule) -> c_uint;
    pub fn clang_Module_getParent(module: CXModule) -> CXModule;
    pub fn clang_Module_getTopLevelHeader(tu: CXTranslationUnit, module: CXModule, index: c_uint) -> CXFile;
    pub fn clang_Module_isSystem(module: CXModule) -> c_int;
    pub fn clang_Range_isNull(range: CXSourceRange) -> c_int;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_TargetInfo_dispose(info: CXTargetInfo);
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_TargetInfo_getPointerWidth(info: CXTargetInfo) -> c_int;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_TargetInfo_getTriple(info: CXTargetInfo) -> CXString;
    pub fn clang_Type_getAlignOf(type_: CXType) -> c_longlong;
    pub fn clang_Type_getCXXRefQualifier(type_: CXType) -> CXRefQualifierKind;
    pub fn clang_Type_getClassType(type_: CXType) -> CXType;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Type_getNamedType(type_: CXType) -> CXType;
    pub fn clang_Type_getNumTemplateArguments(type_: CXType) -> c_int;
    #[cfg(feature="gte_clang_3_9")]
    pub fn clang_Type_getObjCEncoding(type_: CXType) -> CXString;
    pub fn clang_Type_getOffsetOf(type_: CXType, field: *const c_char) -> c_longlong;
    pub fn clang_Type_getSizeOf(type_: CXType) -> c_longlong;
    pub fn clang_Type_getTemplateArgumentAsType(type_: CXType, index: c_uint) -> CXType;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_Type_isTransparentTagTypedef(type_: CXType) -> c_uint;
    #[cfg(feature="gte_clang_3_7")]
    pub fn clang_Type_visitFields(type_: CXType, visitor: CXFieldVisitor, data: CXClientData) -> CXVisitorResult;
    pub fn clang_annotateTokens(tu: CXTranslationUnit, tokens: *mut CXToken, n_tokens: c_uint, cursors: *mut CXCursor);
    pub fn clang_codeCompleteAt(tu: CXTranslationUnit, file: *const c_char, line: c_uint, column: c_uint, unsaved: *mut CXUnsavedFile, n_unsaved: c_uint, flags: CXCodeComplete_Flags) -> *mut CXCodeCompleteResults;
    pub fn clang_codeCompleteGetContainerKind(results: *mut CXCodeCompleteResults, incomplete: *mut c_uint) -> CXCursorKind;
    pub fn clang_codeCompleteGetContainerUSR(results: *mut CXCodeCompleteResults) -> CXString;
    pub fn clang_codeCompleteGetContexts(results: *mut CXCodeCompleteResults) -> c_ulonglong;
    pub fn clang_codeCompleteGetDiagnostic(results: *mut CXCodeCompleteResults, index: c_uint) -> CXDiagnostic;
    pub fn clang_codeCompleteGetNumDiagnostics(results: *mut CXCodeCompleteResults) -> c_uint;
    pub fn clang_codeCompleteGetObjCSelector(results: *mut CXCodeCompleteResults) -> CXString;
    pub fn clang_constructUSR_ObjCCategory(class: *const c_char, category: *const c_char) -> CXString;
    pub fn clang_constructUSR_ObjCClass(class: *const c_char) -> CXString;
    pub fn clang_constructUSR_ObjCIvar(name: *const c_char, usr: CXString) -> CXString;
    pub fn clang_constructUSR_ObjCMethod(name: *const c_char, instance: c_uint, usr: CXString) -> CXString;
    pub fn clang_constructUSR_ObjCProperty(property: *const c_char, usr: CXString) -> CXString;
    pub fn clang_constructUSR_ObjCProtocol(protocol: *const c_char) -> CXString;
    pub fn clang_createCXCursorSet() -> CXCursorSet;
    pub fn clang_createIndex(exclude: c_int, display: c_int) -> CXIndex;
    pub fn clang_createTranslationUnit(index: CXIndex, file: *const c_char) -> CXTranslationUnit;
    pub fn clang_createTranslationUnit2(index: CXIndex, file: *const c_char, tu: *mut CXTranslationUnit) -> CXErrorCode;
    pub fn clang_createTranslationUnitFromSourceFile(index: CXIndex, file: *const c_char, n_arguments: c_int, arguments: *const *const c_char, n_unsaved: c_uint, unsaved: *mut CXUnsavedFile) -> CXTranslationUnit;
    pub fn clang_defaultCodeCompleteOptions() -> CXCodeComplete_Flags;
    pub fn clang_defaultDiagnosticDisplayOptions() -> CXDiagnosticDisplayOptions;
    pub fn clang_defaultEditingTranslationUnitOptions() -> CXTranslationUnit_Flags;
    pub fn clang_defaultReparseOptions(tu: CXTranslationUnit) -> CXReparse_Flags;
    pub fn clang_defaultSaveOptions(tu: CXTranslationUnit) -> CXSaveTranslationUnit_Flags;
    pub fn clang_disposeCXCursorSet(set: CXCursorSet);
    pub fn clang_disposeCXPlatformAvailability(availability: *mut CXPlatformAvailability);
    pub fn clang_disposeCXTUResourceUsage(usage: CXTUResourceUsage);
    pub fn clang_disposeCodeCompleteResults(results: *mut CXCodeCompleteResults);
    pub fn clang_disposeDiagnostic(diagnostic: CXDiagnostic);
    pub fn clang_disposeDiagnosticSet(diagnostic: CXDiagnosticSet);
    pub fn clang_disposeIndex(index: CXIndex);
    pub fn clang_disposeOverriddenCursors(cursors: *mut CXCursor);
    pub fn clang_disposeSourceRangeList(list: *mut CXSourceRangeList);
    pub fn clang_disposeString(string: CXString);
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_disposeStringSet(set: *mut CXStringSet);
    pub fn clang_disposeTokens(tu: CXTranslationUnit, tokens: *mut CXToken, n_tokens: c_uint);
    pub fn clang_disposeTranslationUnit(tu: CXTranslationUnit);
    pub fn clang_enableStackTraces();
    pub fn clang_equalCursors(left: CXCursor, right: CXCursor) -> c_uint;
    pub fn clang_equalLocations(left: CXSourceLocation, right: CXSourceLocation) -> c_uint;
    pub fn clang_equalRanges(left: CXSourceRange, right: CXSourceRange) -> c_uint;
    pub fn clang_equalTypes(left: CXType, right: CXType) -> c_uint;
    pub fn clang_executeOnThread(function: extern fn(*mut c_void), data: *mut c_void, stack: c_uint);
    pub fn clang_findIncludesInFile(tu: CXTranslationUnit, file: CXFile, cursor: CXCursorAndRangeVisitor) -> CXResult;
    pub fn clang_findReferencesInFile(cursor: CXCursor, file: CXFile, visitor: CXCursorAndRangeVisitor) -> CXResult;
    pub fn clang_formatDiagnostic(diagnostic: CXDiagnostic, flags: CXDiagnosticDisplayOptions) -> CXString;
    #[cfg(feature="gte_clang_3_7")]
    pub fn clang_free(buffer: *mut c_void);
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_getAddressSpace(type_: CXType) -> c_uint;
    #[cfg(feature="gte_clang_4_0")]
    pub fn clang_getAllSkippedRanges(tu: CXTranslationUnit) -> *mut CXSourceRangeList;
    pub fn clang_getArgType(type_: CXType, index: c_uint) -> CXType;
    pub fn clang_getArrayElementType(type_: CXType) -> CXType;
    pub fn clang_getArraySize(type_: CXType) -> c_longlong;
    pub fn clang_getCString(string: CXString) -> *const c_char;
    pub fn clang_getCXTUResourceUsage(tu: CXTranslationUnit) -> CXTUResourceUsage;
    pub fn clang_getCXXAccessSpecifier(cursor: CXCursor) -> CX_CXXAccessSpecifier;
    pub fn clang_getCanonicalCursor(cursor: CXCursor) -> CXCursor;
    pub fn clang_getCanonicalType(type_: CXType) -> CXType;
    pub fn clang_getChildDiagnostics(diagnostic: CXDiagnostic) -> CXDiagnosticSet;
    pub fn clang_getClangVersion() -> CXString;
    pub fn clang_getCompletionAnnotation(string: CXCompletionString, index: c_uint) -> CXString;
    pub fn clang_getCompletionAvailability(string: CXCompletionString) -> CXAvailabilityKind;
    pub fn clang_getCompletionBriefComment(string: CXCompletionString) -> CXString;
    pub fn clang_getCompletionChunkCompletionString(string: CXCompletionString, index: c_uint) -> CXCompletionString;
    pub fn clang_getCompletionChunkKind(string: CXCompletionString, index: c_uint) -> CXCompletionChunkKind;
    pub fn clang_getCompletionChunkText(string: CXCompletionString, index: c_uint) -> CXString;
    pub fn clang_getCompletionNumAnnotations(string: CXCompletionString) -> c_uint;
    pub fn clang_getCompletionParent(string: CXCompletionString, kind: *mut CXCursorKind) -> CXString;
    pub fn clang_getCompletionPriority(string: CXCompletionString) -> c_uint;
    pub fn clang_getCursor(tu: CXTranslationUnit, location: CXSourceLocation) -> CXCursor;
    pub fn clang_getCursorAvailability(cursor: CXCursor) -> CXAvailabilityKind;
    pub fn clang_getCursorCompletionString(cursor: CXCursor) -> CXCompletionString;
    pub fn clang_getCursorDefinition(cursor: CXCursor) -> CXCursor;
    pub fn clang_getCursorDisplayName(cursor: CXCursor) -> CXString;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_getCursorExceptionSpecificationType(cursor: CXCursor) -> CXCursor_ExceptionSpecificationKind;
    pub fn clang_getCursorExtent(cursor: CXCursor) -> CXSourceRange;
    pub fn clang_getCursorKind(cursor: CXCursor) -> CXCursorKind;
    pub fn clang_getCursorKindSpelling(kind: CXCursorKind) -> CXString;
    pub fn clang_getCursorLanguage(cursor: CXCursor) -> CXLanguageKind;
    pub fn clang_getCursorLexicalParent(cursor: CXCursor) -> CXCursor;
    pub fn clang_getCursorLinkage(cursor: CXCursor) -> CXLinkageKind;
    pub fn clang_getCursorLocation(cursor: CXCursor) -> CXSourceLocation;
    pub fn clang_getCursorPlatformAvailability(cursor: CXCursor, deprecated: *mut c_int, deprecated_message: *mut CXString, unavailable: *mut c_int, unavailable_message: *mut CXString, availability: *mut CXPlatformAvailability, n_availability: c_int) -> c_int;
    pub fn clang_getCursorReferenceNameRange(cursor: CXCursor, flags: CXNameRefFlags, index: c_uint) -> CXSourceRange;
    pub fn clang_getCursorReferenced(cursor: CXCursor) -> CXCursor;
    pub fn clang_getCursorResultType(cursor: CXCursor) -> CXType;
    pub fn clang_getCursorSemanticParent(cursor: CXCursor) -> CXCursor;
    pub fn clang_getCursorSpelling(cursor: CXCursor) -> CXString;
    pub fn clang_getCursorType(cursor: CXCursor) -> CXType;
    pub fn clang_getCursorUSR(cursor: CXCursor) -> CXString;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_getCursorVisibility(cursor: CXCursor) -> CXVisibilityKind;
    pub fn clang_getDeclObjCTypeEncoding(cursor: CXCursor) -> CXString;
    pub fn clang_getDefinitionSpellingAndExtent(cursor: CXCursor, start: *mut *const c_char, end: *mut *const c_char, start_line: *mut c_uint, start_column: *mut c_uint, end_line: *mut c_uint, end_column: *mut c_uint);
    pub fn clang_getDiagnostic(tu: CXTranslationUnit, index: c_uint) -> CXDiagnostic;
    pub fn clang_getDiagnosticCategory(diagnostic: CXDiagnostic) -> c_uint;
    pub fn clang_getDiagnosticCategoryName(category: c_uint) -> CXString;
    pub fn clang_getDiagnosticCategoryText(diagnostic: CXDiagnostic) -> CXString;
    pub fn clang_getDiagnosticFixIt(diagnostic: CXDiagnostic, index: c_uint, range: *mut CXSourceRange) -> CXString;
    pub fn clang_getDiagnosticInSet(diagnostic: CXDiagnosticSet, index: c_uint) -> CXDiagnostic;
    pub fn clang_getDiagnosticLocation(diagnostic: CXDiagnostic) -> CXSourceLocation;
    pub fn clang_getDiagnosticNumFixIts(diagnostic: CXDiagnostic) -> c_uint;
    pub fn clang_getDiagnosticNumRanges(diagnostic: CXDiagnostic) -> c_uint;
    pub fn clang_getDiagnosticOption(diagnostic: CXDiagnostic, option: *mut CXString) -> CXString;
    pub fn clang_getDiagnosticRange(diagnostic: CXDiagnostic, index: c_uint) -> CXSourceRange;
    pub fn clang_getDiagnosticSetFromTU(tu: CXTranslationUnit) -> CXDiagnosticSet;
    pub fn clang_getDiagnosticSeverity(diagnostic: CXDiagnostic) -> CXDiagnosticSeverity;
    pub fn clang_getDiagnosticSpelling(diagnostic: CXDiagnostic) -> CXString;
    pub fn clang_getElementType(type_: CXType) -> CXType;
    pub fn clang_getEnumConstantDeclUnsignedValue(cursor: CXCursor) -> c_ulonglong;
    pub fn clang_getEnumConstantDeclValue(cursor: CXCursor) -> c_longlong;
    pub fn clang_getEnumDeclIntegerType(cursor: CXCursor) -> CXType;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_getExceptionSpecificationType(type_: CXType) -> CXCursor_ExceptionSpecificationKind;
    pub fn clang_getExpansionLocation(location: CXSourceLocation, file: *mut CXFile, line: *mut c_uint, column: *mut c_uint, offset: *mut c_uint);
    pub fn clang_getFieldDeclBitWidth(cursor: CXCursor) -> c_int;
    pub fn clang_getFile(tu: CXTranslationUnit, file: *const c_char) -> CXFile;
    pub fn clang_getFileLocation(location: CXSourceLocation, file: *mut CXFile, line: *mut c_uint, column: *mut c_uint, offset: *mut c_uint);
    pub fn clang_getFileName(file: CXFile) -> CXString;
    pub fn clang_getFileTime(file: CXFile) -> time_t;
    pub fn clang_getFileUniqueID(file: CXFile, id: *mut CXFileUniqueID) -> c_int;
    pub fn clang_getFunctionTypeCallingConv(type_: CXType) -> CXCallingConv;
    pub fn clang_getIBOutletCollectionType(cursor: CXCursor) -> CXType;
    pub fn clang_getIncludedFile(cursor: CXCursor) -> CXFile;
    pub fn clang_getInclusions(tu: CXTranslationUnit, visitor: CXInclusionVisitor, data: CXClientData);
    pub fn clang_getInstantiationLocation(location: CXSourceLocation, file: *mut CXFile, line: *mut c_uint, column: *mut c_uint, offset: *mut c_uint);
    pub fn clang_getLocation(tu: CXTranslationUnit, file: CXFile, line: c_uint, column: c_uint) -> CXSourceLocation;
    pub fn clang_getLocationForOffset(tu: CXTranslationUnit, file: CXFile, offset: c_uint) -> CXSourceLocation;
    pub fn clang_getModuleForFile(tu: CXTranslationUnit, file: CXFile) -> CXModule;
    pub fn clang_getNullCursor() -> CXCursor;
    pub fn clang_getNullLocation() -> CXSourceLocation;
    pub fn clang_getNullRange() -> CXSourceRange;
    pub fn clang_getNumArgTypes(type_: CXType) -> c_int;
    pub fn clang_getNumCompletionChunks(string: CXCompletionString) -> c_uint;
    pub fn clang_getNumDiagnostics(tu: CXTranslationUnit) -> c_uint;
    pub fn clang_getNumDiagnosticsInSet(diagnostic: CXDiagnosticSet) -> c_uint;
    pub fn clang_getNumElements(type_: CXType) -> c_longlong;
    pub fn clang_getNumOverloadedDecls(cursor: CXCursor) -> c_uint;
    pub fn clang_getOverloadedDecl(cursor: CXCursor, index: c_uint) -> CXCursor;
    pub fn clang_getOverriddenCursors(cursor: CXCursor, cursors: *mut *mut CXCursor, n_cursors: *mut c_uint);
    pub fn clang_getPointeeType(type_: CXType) -> CXType;
    pub fn clang_getPresumedLocation(location: CXSourceLocation, file: *mut CXString, line: *mut c_uint, column: *mut c_uint);
    pub fn clang_getRange(start: CXSourceLocation, end: CXSourceLocation) -> CXSourceRange;
    pub fn clang_getRangeEnd(range: CXSourceRange) -> CXSourceLocation;
    pub fn clang_getRangeStart(range: CXSourceRange) -> CXSourceLocation;
    pub fn clang_getRemappings(file: *const c_char) -> CXRemapping;
    pub fn clang_getRemappingsFromFileList(files: *mut *const c_char, n_files: c_uint) -> CXRemapping;
    pub fn clang_getResultType(type_: CXType) -> CXType;
    pub fn clang_getSkippedRanges(tu: CXTranslationUnit, file: CXFile) -> *mut CXSourceRangeList;
    pub fn clang_getSpecializedCursorTemplate(cursor: CXCursor) -> CXCursor;
    pub fn clang_getSpellingLocation(location: CXSourceLocation, file: *mut CXFile, line: *mut c_uint, column: *mut c_uint, offset: *mut c_uint);
    pub fn clang_getTUResourceUsageName(kind: CXTUResourceUsageKind) -> *const c_char;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_getTranslationUnitTargetInfo(tu: CXTranslationUnit) -> CXTargetInfo;
    pub fn clang_getTemplateCursorKind(cursor: CXCursor) -> CXCursorKind;
    pub fn clang_getTokenExtent(tu: CXTranslationUnit, token: CXToken) -> CXSourceRange;
    pub fn clang_getTokenKind(token: CXToken) -> CXTokenKind;
    pub fn clang_getTokenLocation(tu: CXTranslationUnit, token: CXToken) -> CXSourceLocation;
    pub fn clang_getTokenSpelling(tu: CXTranslationUnit, token: CXToken) -> CXString;
    pub fn clang_getTranslationUnitCursor(tu: CXTranslationUnit) -> CXCursor;
    pub fn clang_getTranslationUnitSpelling(tu: CXTranslationUnit) -> CXString;
    pub fn clang_getTypeDeclaration(type_: CXType) -> CXCursor;
    pub fn clang_getTypeKindSpelling(type_: CXTypeKind) -> CXString;
    pub fn clang_getTypeSpelling(type_: CXType) -> CXString;
    pub fn clang_getTypedefDeclUnderlyingType(cursor: CXCursor) -> CXType;
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_getTypedefName(type_: CXType) -> CXString;
    pub fn clang_hashCursor(cursor: CXCursor) -> c_uint;
    pub fn clang_indexLoc_getCXSourceLocation(location: CXIdxLoc) -> CXSourceLocation;
    pub fn clang_indexLoc_getFileLocation(location: CXIdxLoc, index_file: *mut CXIdxClientFile, file: *mut CXFile, line: *mut c_uint, column: *mut c_uint, offset: *mut c_uint);
    pub fn clang_indexSourceFile(index: CXIndexAction, data: CXClientData, callbacks: *mut IndexerCallbacks, n_callbacks: c_uint, index_flags: CXIndexOptFlags, file: *const c_char, arguments: *const *const c_char, n_arguments: c_int, unsaved: *mut CXUnsavedFile, n_unsaved: c_uint, tu: *mut CXTranslationUnit, tu_flags: CXTranslationUnit_Flags) -> CXErrorCode;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_indexSourceFileFullArgv(index: CXIndexAction, data: CXClientData, callbacks: *mut IndexerCallbacks, n_callbacks: c_uint, index_flags: CXIndexOptFlags, file: *const c_char, arguments: *const *const c_char, n_arguments: c_int, unsaved: *mut CXUnsavedFile, n_unsaved: c_uint, tu: *mut CXTranslationUnit, tu_flags: CXTranslationUnit_Flags) -> CXErrorCode;
    pub fn clang_indexTranslationUnit(index: CXIndexAction, data: CXClientData, callbacks: *mut IndexerCallbacks, n_callbacks: c_uint, flags: CXIndexOptFlags, tu: CXTranslationUnit) -> c_int;
    pub fn clang_index_getCXXClassDeclInfo(info: *const CXIdxDeclInfo) -> *const CXIdxCXXClassDeclInfo;
    pub fn clang_index_getClientContainer(info: *const CXIdxContainerInfo) -> CXIdxClientContainer;
    pub fn clang_index_getClientEntity(info: *const CXIdxEntityInfo) -> CXIdxClientEntity;
    pub fn clang_index_getIBOutletCollectionAttrInfo(info: *const CXIdxAttrInfo) -> *const CXIdxIBOutletCollectionAttrInfo;
    pub fn clang_index_getObjCCategoryDeclInfo(info: *const CXIdxDeclInfo) -> *const CXIdxObjCCategoryDeclInfo;
    pub fn clang_index_getObjCContainerDeclInfo(info: *const CXIdxDeclInfo) -> *const CXIdxObjCContainerDeclInfo;
    pub fn clang_index_getObjCInterfaceDeclInfo(info: *const CXIdxDeclInfo) -> *const CXIdxObjCInterfaceDeclInfo;
    pub fn clang_index_getObjCPropertyDeclInfo(info: *const CXIdxDeclInfo) -> *const CXIdxObjCPropertyDeclInfo;
    pub fn clang_index_getObjCProtocolRefListInfo(info: *const CXIdxDeclInfo) -> *const CXIdxObjCProtocolRefListInfo;
    pub fn clang_index_isEntityObjCContainerKind(info: CXIdxEntityKind) -> c_int;
    pub fn clang_index_setClientContainer(info: *const CXIdxContainerInfo, container: CXIdxClientContainer);
    pub fn clang_index_setClientEntity(info: *const CXIdxEntityInfo, entity: CXIdxClientEntity);
    pub fn clang_isAttribute(kind: CXCursorKind) -> c_uint;
    pub fn clang_isConstQualifiedType(type_: CXType) -> c_uint;
    pub fn clang_isCursorDefinition(cursor: CXCursor) -> c_uint;
    pub fn clang_isDeclaration(kind: CXCursorKind) -> c_uint;
    pub fn clang_isExpression(kind: CXCursorKind) -> c_uint;
    pub fn clang_isFileMultipleIncludeGuarded(tu: CXTranslationUnit, file: CXFile) -> c_uint;
    pub fn clang_isFunctionTypeVariadic(type_: CXType) -> c_uint;
    pub fn clang_isInvalid(kind: CXCursorKind) -> c_uint;
    pub fn clang_isPODType(type_: CXType) -> c_uint;
    pub fn clang_isPreprocessing(kind: CXCursorKind) -> c_uint;
    pub fn clang_isReference(kind: CXCursorKind) -> c_uint;
    pub fn clang_isRestrictQualifiedType(type_: CXType) -> c_uint;
    pub fn clang_isStatement(kind: CXCursorKind) -> c_uint;
    pub fn clang_isTranslationUnit(kind: CXCursorKind) -> c_uint;
    pub fn clang_isUnexposed(kind: CXCursorKind) -> c_uint;
    pub fn clang_isVirtualBase(cursor: CXCursor) -> c_uint;
    pub fn clang_isVolatileQualifiedType(type_: CXType) -> c_uint;
    pub fn clang_loadDiagnostics(file: *const c_char, error: *mut CXLoadDiag_Error, message: *mut CXString) -> CXDiagnosticSet;
    pub fn clang_parseTranslationUnit(index: CXIndex, file: *const c_char, arguments: *const *const c_char, n_arguments: c_int, unsaved: *mut CXUnsavedFile, n_unsaved: c_uint, flags: CXTranslationUnit_Flags) -> CXTranslationUnit;
    pub fn clang_parseTranslationUnit2(index: CXIndex, file: *const c_char, arguments: *const *const c_char, n_arguments: c_int, unsaved: *mut CXUnsavedFile, n_unsaved: c_uint, flags: CXTranslationUnit_Flags, tu: *mut CXTranslationUnit) -> CXErrorCode;
    #[cfg(feature="gte_clang_3_8")]
    pub fn clang_parseTranslationUnit2FullArgv(index: CXIndex, file: *const c_char, arguments: *const *const c_char, n_arguments: c_int, unsaved: *mut CXUnsavedFile, n_unsaved: c_uint, flags: CXTranslationUnit_Flags, tu: *mut CXTranslationUnit) -> CXErrorCode;
    pub fn clang_remap_dispose(remapping: CXRemapping);
    pub fn clang_remap_getFilenames(remapping: CXRemapping, index: c_uint, original: *mut CXString, transformed: *mut CXString);
    pub fn clang_remap_getNumFiles(remapping: CXRemapping) -> c_uint;
    pub fn clang_reparseTranslationUnit(tu: CXTranslationUnit, n_unsaved: c_uint, unsaved: *mut CXUnsavedFile, flags: CXReparse_Flags) -> CXErrorCode;
    pub fn clang_saveTranslationUnit(tu: CXTranslationUnit, file: *const c_char, options: CXSaveTranslationUnit_Flags) -> CXSaveError;
    pub fn clang_sortCodeCompletionResults(results: *mut CXCompletionResult, n_results: c_uint);
    #[cfg(feature="gte_clang_5_0")]
    pub fn clang_suspendTranslationUnit(tu: CXTranslationUnit) -> c_uint;
    pub fn clang_toggleCrashRecovery(recovery: c_uint);
    pub fn clang_tokenize(tu: CXTranslationUnit, range: CXSourceRange, tokens: *mut *mut CXToken, n_tokens: *mut c_uint);
    pub fn clang_visitChildren(cursor: CXCursor, visitor: CXCursorVisitor, data: CXClientData) -> c_uint;

    // Documentation
    pub fn clang_BlockCommandComment_getArgText(comment: CXComment, index: c_uint) -> CXString;
    pub fn clang_BlockCommandComment_getCommandName(comment: CXComment) -> CXString;
    pub fn clang_BlockCommandComment_getNumArgs(comment: CXComment) -> c_uint;
    pub fn clang_BlockCommandComment_getParagraph(comment: CXComment) -> CXComment;
    pub fn clang_Comment_getChild(comment: CXComment, index: c_uint) -> CXComment;
    pub fn clang_Comment_getKind(comment: CXComment) -> CXCommentKind;
    pub fn clang_Comment_getNumChildren(comment: CXComment) -> c_uint;
    pub fn clang_Comment_isWhitespace(comment: CXComment) -> c_uint;
    pub fn clang_Cursor_getParsedComment(C: CXCursor) -> CXComment;
    pub fn clang_FullComment_getAsHTML(comment: CXComment) -> CXString;
    pub fn clang_FullComment_getAsXML(comment: CXComment) -> CXString;
    pub fn clang_HTMLStartTagComment_isSelfClosing(comment: CXComment) -> c_uint;
    pub fn clang_HTMLStartTag_getAttrName(comment: CXComment, index: c_uint) -> CXString;
    pub fn clang_HTMLStartTag_getAttrValue(comment: CXComment, index: c_uint) -> CXString;
    pub fn clang_HTMLStartTag_getNumAttrs(comment: CXComment) -> c_uint;
    pub fn clang_HTMLTagComment_getAsString(comment: CXComment) -> CXString;
    pub fn clang_HTMLTagComment_getTagName(comment: CXComment) -> CXString;
    pub fn clang_InlineCommandComment_getArgText(comment: CXComment, index: c_uint) -> CXString;
    pub fn clang_InlineCommandComment_getCommandName(comment: CXComment) -> CXString;
    pub fn clang_InlineCommandComment_getNumArgs(comment: CXComment) -> c_uint;
    pub fn clang_InlineCommandComment_getRenderKind(comment: CXComment) -> CXCommentInlineCommandRenderKind;
    pub fn clang_InlineContentComment_hasTrailingNewline(comment: CXComment) -> c_uint;
    pub fn clang_ParamCommandComment_getDirection(comment: CXComment) -> CXCommentParamPassDirection;
    pub fn clang_ParamCommandComment_getParamIndex(comment: CXComment) -> c_uint;
    pub fn clang_ParamCommandComment_getParamName(comment: CXComment) -> CXString;
    pub fn clang_ParamCommandComment_isDirectionExplicit(comment: CXComment) -> c_uint;
    pub fn clang_ParamCommandComment_isParamIndexValid(comment: CXComment) -> c_uint;
    pub fn clang_TParamCommandComment_getDepth(comment: CXComment) -> c_uint;
    pub fn clang_TParamCommandComment_getIndex(comment: CXComment, depth: c_uint) -> c_uint;
    pub fn clang_TParamCommandComment_getParamName(comment: CXComment) -> CXString;
    pub fn clang_TParamCommandComment_isParamPositionValid(comment: CXComment) -> c_uint;
    pub fn clang_TextComment_getText(comment: CXComment) -> CXString;
    pub fn clang_VerbatimBlockLineComment_getText(comment: CXComment) -> CXString;
    pub fn clang_VerbatimLineComment_getText(comment: CXComment) -> CXString;
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
