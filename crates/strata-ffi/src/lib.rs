//! Everything that crosses the Rust/C++ boundary. See the crate's README for the rules.

use std::ffi::{CStr, OsString, c_char, c_int};
use std::os::unix::ffi::OsStringExt;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub mod bridge;
pub mod bridge_modlist;
pub mod bridge_builtins;
pub mod bridge_crashlogs;
pub mod bridge_conflicts;
pub mod bridge_install;
pub mod bridge_loot;
pub mod bridge_instances;

/// The C++ shell's entry: creates the `QApplication` and the window, returns the exit code.
/// `instance` and `profile` are what `-i` and `-p` gave, as NUL-terminated strings, empty
/// when not given.
pub type ShellEntry =
    unsafe extern "C" fn(argc: c_int, argv: *mut *mut c_char, instance: *const c_char, profile: *const c_char) -> c_int;

/// A command-line value for C: NUL-terminated, empty when absent (an interior NUL, which a
/// command line cannot hold, also gives empty).
fn c_string(value: Option<&std::ffi::OsStr>) -> std::ffi::CString {
    use std::os::unix::ffi::OsStrExt;
    value.and_then(|v| std::ffi::CString::new(v.as_bytes()).ok()).unwrap_or_default()
}

/// Runs a bridge function that returns a `Result`; a panic becomes the error
/// `internal error: <message>` instead of unwinding into C++ (undefined behaviour).
pub(crate) fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Err(format!("internal error: {message}"))
    })
}

/// Runs a bridge function without a `Result`; a panic gives the empty value.
pub(crate) fn quiet<T: Default>(f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_default()
}

/// Exit code when Rust panicked: the panic message is already on stderr.
const EXIT_PANIC: c_int = 101;

/// The program's real entry, called by `shell/src/main.cpp` with `main`'s own arguments.
///
/// # Safety
/// `argv` must point to `argc` valid NUL-terminated strings that live until this returns
/// (as `main`'s do). `shell`, when given, is called at most once, on this thread, with the
/// same `argc` and `argv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strata_run(
    argc: c_int,
    argv: *mut *mut c_char,
    shell: Option<ShellEntry>,
) -> c_int {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let count = usize::try_from(argc).unwrap_or(0);
        let args: Vec<OsString> = (0..count)
            .map(|i| {
                // SAFETY: the caller guarantees `argc` valid strings.
                let arg = unsafe { CStr::from_ptr(*argv.add(i)) };
                OsString::from_vec(arg.to_bytes().to_vec())
            })
            .collect();

        match shell {
            // SAFETY: the caller guarantees `shell` may be called with these arguments.
            Some(entry) => strata_cli::run(
                args,
                Some(&|window: &strata_cli::WindowArgs| {
                    let instance = c_string(window.instance.as_deref().map(|p| p.as_os_str()));
                    let profile = c_string(window.profile.as_deref().map(std::ffi::OsStr::new));
                    // SAFETY: the strings live until the call returns.
                    unsafe { entry(argc, argv, instance.as_ptr(), profile.as_ptr()) }
                }),
            ),
            None => strata_cli::run(args, None),
        }
    }));
    outcome.unwrap_or(EXIT_PANIC)
}
