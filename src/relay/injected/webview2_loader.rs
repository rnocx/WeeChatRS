//! Microsoft's WebView2 loader, linked into MinGW builds so that the .exe doesn't need
//! WebView2Loader.dll next to it.
//!
//! webview2-com-sys links its static loader with MSVC only, and the DLL's import library
//! with MinGW. Its build script puts both on the link path though, and the linker takes
//! a function from the first library that has it: naming the static one here puts it
//! ahead of the import library, which is then left out.
//!
//! The static loader is compiled for the Visual C++ runtime. What it needs from there
//! and MinGW lacks is below; a WebView2 update that needs more fails to link.
#![allow(non_snake_case, non_upper_case_globals)]

use std::ffi::c_void;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

#[link(name = "WebView2LoaderStatic", kind = "static", modifiers = "-bundle")]
extern "system" {
    fn CompareBrowserVersions();
    fn CreateCoreWebView2Environment();
    fn CreateCoreWebView2EnvironmentWithOptions();
    fn GetAvailableCoreWebView2BrowserVersionString();
    fn GetAvailableCoreWebView2BrowserVersionStringWithOptions();
}

// What the loader calls in Windows itself.
#[link(name = "advapi32")]
#[link(name = "ole32")]
extern "system" {}

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

/// Never called: this makes the linker look for the functions before it gets to the
/// import library.
#[used]
static LOADER: [unsafe extern "system" fn(); 5] = [
    CompareBrowserVersions,
    CreateCoreWebView2Environment,
    CreateCoreWebView2EnvironmentWithOptions,
    GetAvailableCoreWebView2BrowserVersionString,
    GetAvailableCoreWebView2BrowserVersionStringWithOptions,
];

/// Before anything calls the loader. The Visual C++ startup code does this otherwise.
pub fn init() {
    __security_cookie.store(RandomState::new().build_hasher().finish(), Ordering::Relaxed);
}

// Stack cookies (/GS).
#[no_mangle]
static __security_cookie: AtomicU64 = AtomicU64::new(0x2B99_2DDF_A232);

// Thread-safe initialization of C++ function statics. The compiled code calls the header
// when `once` is above the thread's _Init_thread_epoch, and initializes the static and
// calls the footer if that left `once` at INITIALIZING. The Visual C++ runtime raises the
// epoch to skip later calls; here it stays at i32::MIN, so every pass calls the header.
const UNINITIALIZED: i32 = 0;
const INITIALIZING: i32 = -1;
const INITIALIZED: i32 = i32::MIN + 1;

#[no_mangle]
extern "C" fn _Init_thread_header(once: &AtomicI32) {
    while once.compare_exchange(UNINITIALIZED, INITIALIZING, Ordering::Acquire, Ordering::Acquire)
        == Err(INITIALIZING)
    {
        std::thread::yield_now();
    }
}

#[no_mangle]
extern "C" fn _Init_thread_footer(once: &AtomicI32) {
    once.store(INITIALIZED, Ordering::Release);
}

// The C++ operators new and delete.

/// `operator new(size_t, const std::nothrow_t&)`
#[export_name = "??2@YAPEAX_KAEBUnothrow_t@std@@@Z"]
extern "C" fn new_nothrow(size: usize, _: &u8) -> *mut c_void {
    unsafe { malloc(size.max(1)) }
}

/// `operator new[](size_t)`, which must not return null.
#[export_name = "??_U@YAPEAX_K@Z"]
extern "C" fn new_array(size: usize) -> *mut c_void {
    let ptr = unsafe { malloc(size.max(1)) };
    if ptr.is_null() {
        std::process::abort();
    }
    ptr
}

/// `operator delete(void*, size_t)`
#[export_name = "??3@YAXPEAX_K@Z"]
unsafe extern "C" fn delete_sized(ptr: *mut c_void, _: usize) {
    free(ptr);
}

/// `operator delete[](void*)`
#[export_name = "??_V@YAXPEAX@Z"]
unsafe extern "C" fn delete_array(ptr: *mut c_void) {
    free(ptr);
}

/// `std::nothrow`
#[export_name = "?nothrow@std@@3Unothrow_t@1@B"]
static NOTHROW: u8 = 0;

// A thread-local, the stack cookie check, which keeps the registers its caller is using,
// and the control flow guard's check of a call through a pointer, which checks nothing:
// the program isn't built with the guard. Newer MinGW runtimes have these pointers too,
// in one object file, so all of them are here to keep that one from being linked as well.
#[cfg(target_arch = "x86_64")]
std::arch::global_asm!(
    ".section .tls$,\"dw\"",
    ".globl _Init_thread_epoch",
    ".p2align 2",
    "_Init_thread_epoch:",
    ".long 0x80000000",
    ".text",
    ".globl __security_check_cookie",
    "__security_check_cookie:",
    "cmp rcx, [rip + __security_cookie]",
    "jne 1f",
    "ret",
    "1: mov ecx, 2", // FAST_FAIL_STACK_COOKIE_CHECK_FAILURE
    "int 0x29",
    "webview2_loader_call:",
    "jmp rax",
    "webview2_loader_check:",
    "ret",
    ".section .rdata,\"dr\"",
    ".globl __guard_dispatch_icall_fptr, __guard_check_icall_fptr",
    ".p2align 3",
    "__guard_dispatch_icall_fptr:",
    ".quad webview2_loader_call",
    "__guard_check_icall_fptr:",
    ".quad webview2_loader_check",
    ".text",
);

#[cfg(target_arch = "aarch64")]
std::arch::global_asm!(
    ".section .tls$,\"dw\"",
    ".globl _Init_thread_epoch",
    ".p2align 2",
    "_Init_thread_epoch:",
    ".long 0x80000000",
    ".text",
    ".globl __security_check_cookie",
    ".p2align 2",
    "__security_check_cookie:",
    "adrp x16, __security_cookie",
    "ldr x16, [x16, :lo12:__security_cookie]",
    "cmp x0, x16",
    "b.ne 1f",
    "ret",
    "1: mov w0, #2", // FAST_FAIL_STACK_COOKIE_CHECK_FAILURE
    "brk #0xf003",
    "webview2_loader_check:",
    "ret",
    ".section .rdata,\"dr\"",
    ".globl __guard_check_icall_fptr",
    ".p2align 3",
    "__guard_check_icall_fptr:",
    ".quad webview2_loader_check",
    ".text",
);
