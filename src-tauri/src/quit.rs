//! "Close Spotify for me" (#43) for a Spotify started without Pantry's port: brings Spotify's window to the front
//! and sends its own quit shortcut, Ctrl+Shift+Q (the same as File → Exit). Never a forced kill.
//! Guards: Pantry must be the active window (only after a click in Pantry), and the keys are sent only if the
//! active window then belongs to Spotify.exe, so they can't reach another app.

use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Pantry isn't the active window, so Windows won't let it bring Spotify to the front.
    NotActive,
    NoWindow,
    /// Something else came to the front: nothing was sent.
    FocusLost,
    /// The keys were sent, but Spotify is still running.
    StillRunning,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refused::NotActive => "Pantry has to be the active window for this. Click into Pantry, then try again.",
            Refused::NoWindow => "Couldn't find Spotify's window. Exit it via ⋯ → File → Exit, or right-click its tray icon → Quit.",
            Refused::FocusLost => "Spotify didn't come to the front, so Pantry sent nothing. Try again, or exit it via ⋯ → File → Exit.",
            Refused::StillRunning => "Spotify didn't exit. Exit it via ⋯ → File → Exit, or right-click its tray icon → Quit.",
        })
    }
}

/// `exe` is Spotify.exe: starting it again while Spotify runs makes Spotify show its own window (from the tray too).
#[cfg(windows)]
pub fn quit_spotify(exe: Option<&std::path::Path>, timeout: Duration) -> Result<(), Refused> {
    use std::time::Instant;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    if !crate::spotify::is_running() {
        return Ok(());
    }
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || win::pid_of(fg) != GetCurrentProcessId() {
            return Err(Refused::NotActive);
        }
        let Some(mut h) = win::spotify_window() else { return Err(Refused::NoWindow) };
        let mut was_hidden = IsWindowVisible(h) == 0;
        if was_hidden {
            // In the tray: let Spotify bring its own window back, so its page gets keyboard focus.
            // A window forced visible from outside ignores the shortcut.
            if let Some(exe) = exe {
                let _ = std::process::Command::new(exe).spawn();
                let t0 = Instant::now();
                while t0.elapsed() < Duration::from_secs(8) {
                    std::thread::sleep(Duration::from_millis(300));
                    if let Some(v) = win::spotify_window().filter(|w| IsWindowVisible(*w) != 0) {
                        h = v;
                        was_hidden = false;
                        break;
                    }
                }
            }
            if was_hidden {
                ShowWindow(h, SW_SHOW);
            }
        }
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        // Spotify may have taken the foreground itself; Pantry's right to hand it over expires once something else is active.
        if !win::is_spotify(win::pid_of(GetForegroundWindow())) {
            SetForegroundWindow(h);
        }
        // Give the page time to take keyboard focus after showing or restoring.
        std::thread::sleep(Duration::from_millis(800));
        // The last check before sending: the active window must be Spotify's.
        let now = GetForegroundWindow();
        if now.is_null() || !win::is_spotify(win::pid_of(now)) {
            if was_hidden {
                ShowWindow(h, SW_HIDE);
            }
            return Err(Refused::FocusLost);
        }
        win::send_ctrl_shift_q();
    }
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if !crate::spotify::is_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(Refused::StillRunning)
}

#[cfg(not(windows))]
pub fn quit_spotify(_exe: Option<&std::path::Path>, _timeout: Duration) -> Result<(), Refused> {
    Err(Refused::NoWindow)
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub unsafe fn pid_of(h: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, &mut pid) };
        pid
    }

    pub fn is_spotify(pid: u32) -> bool {
        unsafe {
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if p.is_null() {
                return false;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(p);
            ok && super::exe_is_spotify(&String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    unsafe fn text(h: HWND, class: bool) -> String {
        let mut buf = [0u16; 512];
        let n = unsafe { if class { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) } else { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) } };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    /// Spotify's main window: a Chromium top-level window with a title (the track, or "Spotify Premium").
    /// A visible one wins over a hidden one (hidden = closed to the tray).
    pub fn spotify_window() -> Option<HWND> {
        unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> i32 {
            let found = unsafe { &mut *(lp as *mut Vec<(HWND, bool)>) };
            unsafe {
                if super::is_main_window(&text(h, true), &text(h, false)) && is_spotify(pid_of(h)) {
                    found.push((h, IsWindowVisible(h) != 0));
                }
            }
            1
        }
        let mut found: Vec<(HWND, bool)> = vec![];
        unsafe { EnumWindows(Some(each), &mut found as *mut _ as LPARAM) };
        found.iter().find(|(_, v)| *v).or(found.first()).map(|(h, _)| *h)
    }

    pub unsafe fn send_ctrl_shift_q() {
        let key = |vk: u16, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
        };
        let q = b'Q' as u16;
        let inputs = [key(VK_CONTROL, false), key(VK_SHIFT, false), key(q, false), key(q, true), key(VK_SHIFT, true), key(VK_CONTROL, true)];
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    }
}

fn exe_is_spotify(path: &str) -> bool {
    path.rsplit(['\\', '/']).next().is_some_and(|f| f.eq_ignore_ascii_case("spotify.exe"))
}

fn is_main_window(class: &str, title: &str) -> bool {
    class.starts_with("Chrome_WidgetWin") && !title.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_spotify_and_its_main_window() {
        assert!(exe_is_spotify(r"C:\Users\VC\AppData\Roaming\Spotify\Spotify.exe"));
        assert!(!exe_is_spotify(r"C:\Program Files\Mozilla Firefox\firefox.exe"));
        assert!(!exe_is_spotify(r"C:\x\NotSpotify.exe"));
        assert!(is_main_window("Chrome_WidgetWin_1", "Spotify Premium"));
        assert!(!is_main_window("Chrome_WidgetWin_1", ""));
        assert!(!is_main_window("OleMainThreadWndClass", "OleMainThreadWndName"));
    }

    #[test]
    fn refuses_when_pantry_is_not_active() {
        // Under the test runner Pantry has no window, so it's never the active one: nothing is touched.
        if crate::spotify::is_running() {
            assert_eq!(quit_spotify(None, Duration::from_secs(1)), Err(Refused::NotActive));
        }
    }
}
