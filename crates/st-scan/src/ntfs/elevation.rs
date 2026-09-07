//! Administrator-rights detection and re-launch.
//!
//! Reading a volume's raw bytes requires elevation. Rather than failing,
//! the app checks up front so it can explain why it's asking before a
//! UAC dialog appears, and fall back to the directory walker if the
//! answer is no.

use std::io;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_NORMAL;

/// Whether this process is running elevated.
pub fn is_elevated() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a valid out-param; the pseudo-handle from
    // GetCurrentProcess needs no cleanup.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }

    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned: u32 = 0;
    // SAFETY: the buffer matches the size passed and the class requested.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    // SAFETY: `token` came from OpenProcessToken and is closed once.
    unsafe { CloseHandle(token) };

    ok != 0 && elevation.TokenIsElevated != 0
}

/// Re-launch this executable elevated, triggering a UAC prompt.
///
/// Returns `Ok(false)` when the user declines the prompt — a choice, not
/// a failure, so the caller keeps running unelevated on the walker.
pub fn relaunch_elevated(args: &[String]) -> io::Result<bool> {
    let exe = std::env::current_exe()?;
    let exe_wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = std::ffi::OsStr::new("runas")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let params = args.join(" ");
    let params_wide: Vec<u16> = std::ffi::OsStr::new(&params)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: every pointer is a NUL-terminated UTF-16 string that
    // outlives the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe_wide.as_ptr(),
            if params.is_empty() {
                std::ptr::null()
            } else {
                params_wide.as_ptr()
            },
            std::ptr::null(),
            SW_NORMAL,
        )
    };

    // ShellExecuteW reports success as a value above 32; below that it's
    // an error code, and ERROR_CANCELLED (5) means the user said no.
    const SE_ERR_ACCESSDENIED: isize = 5;
    let code = result as isize;
    if code > 32 {
        Ok(true)
    } else if code == SE_ERR_ACCESSDENIED {
        Ok(false)
    } else {
        Err(io::Error::other(format!(
            "ShellExecuteW failed with code {code}"
        )))
    }
}
