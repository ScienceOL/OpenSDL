//! Shared spawning policy for persistent local processes.
use std::process::{Child, Command};

pub fn spawn(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        prevent_inherited_stdio()?;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{GetFileType, FILE_TYPE_DISK, FILE_TYPE_PIPE},
    System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
};

#[cfg(windows)]
fn prevent_inherited_stdio() -> std::io::Result<()> {
    // Reference: rust-lang/rust b940084d7, std/src/sys/process/windows.rs.
    // Rust's Windows CreateProcessW path inherits all inheritable handles,
    // including the caller's capture pipes even when child stdio is redirected.
    // Clear only the parent's existing stdio handles. Command creates fresh
    // inheritable duplicates for the explicitly selected worker log handles.
    // This is idempotent and intentionally does not restore inheritance flags.
    for stream in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: only query OS-owned stdio handles and alter their inheritance bit.
        // No ownership, pointer dereference, or handle lifetime is transferred.
        unsafe {
            let handle = GetStdHandle(stream);
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            if matches!(GetFileType(handle), FILE_TYPE_DISK | FILE_TYPE_PIPE)
                && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok(())
}
