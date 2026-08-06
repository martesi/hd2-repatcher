//! Windows console detection for the single-binary CLI. The app is built as a
//! windows-subsystem binary (no console), so a windows-subsystem process only
//! gets working `println!`/`stdin` if it explicitly attaches to one -
//! `AttachConsole` is that check: it succeeds when launched from a real
//! terminal, and fails when there is none (double-click, drag-and-drop onto
//! the exe). The no-console case never allocates a fresh one; it hands off to
//! the GUI instead (see `main.rs`), so there is nothing here to free/pause.
//! Always "attached" on non-Windows, where this whole console/subsystem
//! distinction doesn't exist and stdio just works.

#[cfg(windows)]
pub fn has_console() -> bool {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

#[cfg(not(windows))]
pub fn has_console() -> bool {
    true
}
