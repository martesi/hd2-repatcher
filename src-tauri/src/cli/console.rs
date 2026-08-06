//! Windows console handling for the single-binary CLI, replicating the two-exe
//! behaviour of the Python tool. The app is built as a windows-subsystem binary
//! (no console), so in CLI mode we attach to the parent terminal when there is
//! one, or allocate our own console (and pause on exit) when launched by
//! double-click / drag-and-drop. No-ops on non-Windows.
//!
//! `AttachConsole`/`AllocConsole` set the process standard handles for a
//! previously console-less process, so `println!`/`stdin` work afterwards
//! without reopening `CONOUT$`/`CONIN$` by hand.

/// Attaches to the parent console if present, otherwise allocates a fresh one.
/// Returns `true` when we own a freshly allocated console (double-click / drag),
/// meaning the caller should [`pause`] before exiting so output stays readable.
#[cfg(windows)]
pub fn attach_or_alloc() -> bool {
    use windows_sys::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            false
        } else {
            AllocConsole();
            true
        }
    }
}

#[cfg(not(windows))]
pub fn attach_or_alloc() -> bool {
    false
}

/// Holds a self-owned console window open until the user presses Enter, so the
/// result of a double-clicked / dragged run does not vanish.
#[cfg(windows)]
pub fn pause() {
    use std::io::Write;
    print!("\nPress Enter to exit...");
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
}

#[cfg(not(windows))]
pub fn pause() {}
