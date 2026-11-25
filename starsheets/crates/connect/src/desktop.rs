//! What both desktop programs need from the system besides the server link: the Documents folder
//! as Windows and macOS know it, and the terminal on Windows.

use std::path::PathBuf;

/// The user's Documents folder as the system knows it, or `None` where the programs work it out
/// themselves (Linux: `XDG_DOCUMENTS_DIR` and `user-dirs.dirs`).
///
/// Windows: the Documents Known Folder (`SHGetKnownFolderPath`), which follows a move to OneDrive
/// ("Documents backup") or to another drive, unlike `%USERPROFILE%\Documents`. macOS: `~/Documents`.
pub fn system_documents_dir() -> Option<PathBuf> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        dirs::document_dir()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Lets `--version`, `--help` and command-line errors print in the terminal that started the
/// program, on Windows. Release builds there are windowed programs (`windows_subsystem`, so no
/// console window opens beside them), which have no terminal unless they join their parent's.
/// Call it only for runs that print and end: a window joined to a terminal closes with it.
/// Output that already goes somewhere (a pipe or a file) is left alone. Elsewhere it does nothing.
pub fn attach_terminal() {
    #[cfg(windows)]
    win::attach_terminal();
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    fn missing(h: HANDLE) -> bool {
        h.is_null() || h == INVALID_HANDLE_VALUE
    }

    pub(super) fn attach_terminal() {
        let streams: [STD_HANDLE; 2] = [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
        // SAFETY: Win32 calls with constants and a NUL-terminated name that outlives the call; the
        // handles they return are only checked and handed to SetStdHandle.
        unsafe {
            if !streams.iter().any(|&s| missing(GetStdHandle(s))) {
                return;
            }
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return; // started from Explorer or a shortcut: there's no terminal to join
            }
            // Rust's stdout and stderr ask for the standard handles on every write.
            let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
            let out = CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if missing(out) {
                return;
            }
            for s in streams {
                if missing(GetStdHandle(s)) {
                    SetStdHandle(s, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_documents_folder_comes_from_the_system_on_windows_and_macos() {
        let d = super::system_documents_dir();
        if cfg!(any(windows, target_os = "macos")) {
            assert!(d.as_ref().is_some_and(|d| d.is_absolute()), "{d:?}");
        } else {
            assert_eq!(d, None, "Linux reads XDG user-dirs in the programs");
        }
    }

    /// Moves the Documents Known Folder the way OneDrive's "Documents backup" does (the
    /// `Personal` value under User Shell Folders), and checks it's followed. It changes the
    /// account's settings, so it runs only on a CI runner (GITHUB_ACTIONS), never on someone's own
    /// computer, and puts the old value back.
    #[cfg(windows)]
    #[test]
    fn a_documents_folder_moved_to_onedrive_is_followed() {
        if std::env::var_os("GITHUB_ACTIONS").is_none() {
            return;
        }
        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
        let reg = |args: &[&str]| std::process::Command::new("reg").args(args).output().unwrap();
        let before = String::from_utf8_lossy(&reg(&["query", KEY, "/v", "Personal"]).stdout).into_owned();
        let old = before.lines().find_map(|l| l.split("REG_EXPAND_SZ").nth(1).or_else(|| l.split("REG_SZ").nth(1))).map(|v| v.trim().to_string());
        let moved = std::env::temp_dir().join("OneDrive").join("Documents");
        std::fs::create_dir_all(&moved).unwrap();
        let moved_s = moved.to_string_lossy().into_owned();
        assert!(reg(&["add", KEY, "/v", "Personal", "/t", "REG_EXPAND_SZ", "/d", &moved_s, "/f"]).status.success());
        let found = super::system_documents_dir();
        let _ = match &old {
            Some(v) => reg(&["add", KEY, "/v", "Personal", "/t", "REG_EXPAND_SZ", "/d", v, "/f"]),
            None => reg(&["delete", KEY, "/v", "Personal", "/f"]),
        };
        assert_eq!(found.map(|f| f.to_string_lossy().to_lowercase()), Some(moved_s.to_lowercase()));
    }
}
