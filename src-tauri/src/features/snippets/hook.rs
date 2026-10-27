//! Windows: expanding snippet keywords as you type.
//!
//! Low-level keyboard and mouse hooks run on their own high-priority
//! thread. Windows waits for a hook before the key reaches any app, and
//! drops hooks that answer late, so the callbacks only note the key (and
//! which modifiers are held) and pass it to a worker thread. They never
//! block, and skip keys the launcher injects (tagged `INJECTED_TAG`).
//!
//! The worker turns keys into characters with the keyboard layout of the
//! window in front (`ToUnicodeEx`), feeds them to `typing::Typing`, and
//! when a keyword is complete: erases it with Backspace, pastes the
//! snippet through the clipboard (marked private), and puts back what the
//! clipboard held. Typing in the launcher itself is ignored.

use std::cell::RefCell;
use std::ptr::null_mut;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, SendInput, ToUnicodeEx, INPUT, INPUT_0,
    INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_LWIN, VK_MENU,
    VK_PACKET, VK_RWIN, VK_SHIFT, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer,
    PeekMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION,
    HHOOK, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_TIMER,
    WM_XBUTTONDOWN,
};

use super::expand;
use super::typing::{Expansion, Found, Input, Typed, Typing};
use crate::features::clipboard::system::{self, Content};
use crate::platform::INJECTED_TAG;

/// How often the hooks are re-installed: Windows silently removes a hook
/// that once answered too slowly (see D-025).
const REINSTALL_MS: u32 = 10_000;
/// How long to wait for you to let go of Shift, Ctrl, Alt or Win before
/// pressing keys, so they don't mix in.
const MODIFIER_WAIT: Duration = Duration::from_millis(600);
/// How long the app gets to read the snippet off the clipboard before
/// the old content goes back. Apps paste when they get to the keys, so a
/// busy one takes a while; too short and it pastes the old content.
const RESTORE_DELAY: Duration = Duration::from_millis(750);
/// Leave the keyboard's dead-key state alone, so a pending accent still
/// reaches the app (Windows 10 1607+).
const TO_UNICODE_KEEP_STATE: u32 = 4;

#[derive(Debug, Clone, Copy)]
struct Modifiers {
    shift: bool,
    ctrl: bool,
    alt: bool,
    win: bool,
    caps_lock: bool,
}

impl Modifiers {
    fn now() -> Self {
        // SAFETY: no preconditions. High bit set = down; low bit of
        // GetKeyState = toggled on.
        unsafe {
            let down = |vk: u16| GetAsyncKeyState(i32::from(vk)) < 0;
            Self {
                shift: down(VK_SHIFT),
                ctrl: down(VK_CONTROL),
                alt: down(VK_MENU),
                win: down(VK_LWIN) || down(VK_RWIN),
                caps_lock: GetKeyState(i32::from(VK_CAPITAL)) & 1 != 0,
            }
        }
    }
}

enum Event {
    Key {
        vk: u32,
        scan: u32,
        modifiers: Modifiers,
        window: isize,
    },
    Click,
    Update(Vec<Expansion>),
    Stop,
}

/// Runs expansion as you type until dropped.
pub struct Expander {
    hook_thread: u32,
    hook: Option<JoinHandle<()>>,
    events: mpsc::Sender<Event>,
}

impl Expander {
    pub fn start(expansions: Vec<Expansion>) -> Result<Self, String> {
        let (events, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name("snippet-expander".into())
            .spawn(move || run_worker(inbox, expansions))
            .map_err(|e| e.to_string())?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let hook_events = events.clone();
        let hook = std::thread::Builder::new()
            .name("snippet-hook".into())
            .spawn(move || run_hooks(hook_events, ready_tx))
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(hook_thread)) => Ok(Self {
                hook_thread,
                hook: Some(hook),
                events,
            }),
            Ok(Err(e)) => {
                let _ = events.send(Event::Stop);
                let _ = hook.join();
                Err(e)
            }
            Err(_) => {
                let _ = events.send(Event::Stop);
                Err("the keyboard hook thread stopped".into())
            }
        }
    }

    fn update(&self, expansions: Vec<Expansion>) {
        let _ = self.events.send(Event::Update(expansions));
    }
}

impl Drop for Expander {
    fn drop(&mut self) {
        // SAFETY: posting to a thread id fails harmlessly if it has exited.
        unsafe { PostThreadMessageW(self.hook_thread, WM_QUIT, 0, 0) };
        if let Some(hook) = self.hook.take() {
            let _ = hook.join();
        }
        // The worker may be mid-paste; it stops after that by itself.
        let _ = self.events.send(Event::Stop);
    }
}

/// Starts, updates or stops expansion to match `wanted` (none = off).
pub fn apply(running: Option<Expander>, wanted: &[Expansion]) -> Option<Expander> {
    if wanted.is_empty() {
        if running.is_some() {
            tracing::info!("snippet expansion as you type is off");
        }
        return None;
    }
    match running {
        Some(expander) => {
            expander.update(wanted.to_vec());
            Some(expander)
        }
        None => match Expander::start(wanted.to_vec()) {
            Ok(expander) => {
                tracing::info!("expanding {} snippet keywords as you type", wanted.len());
                Some(expander)
            }
            Err(e) => {
                crate::toast::report(format!("Snippets won't expand as you type: {e}"));
                None
            }
        },
    }
}

thread_local! {
    /// Where the hook callbacks send what they see (the hook thread's own).
    static EVENTS: RefCell<Option<mpsc::Sender<Event>>> = const { RefCell::new(None) };
}

fn run_hooks(events: mpsc::Sender<Event>, ready: mpsc::Sender<Result<u32, String>>) {
    EVENTS.with(|e| *e.borrow_mut() = Some(events));
    // SAFETY: standard Win32 hooks + message loop, all on this thread; the
    // hook procedures only run while it waits in GetMessageW.
    unsafe {
        if SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) == 0 {
            tracing::debug!("could not raise the snippet hook's priority");
        }
        let mut msg: MSG = std::mem::zeroed();
        // Create the message queue now, so an early WM_QUIT isn't lost.
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
        let mut hooks = install();
        if hooks.0.is_null() || hooks.1.is_null() {
            let err = std::io::Error::last_os_error().to_string();
            uninstall(hooks);
            let _ = ready.send(Err(err));
            return;
        }
        let _ = ready.send(Ok(GetCurrentThreadId()));
        let timer = SetTimer(null_mut(), 0, REINSTALL_MS, None);
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER && msg.hwnd.is_null() {
                uninstall(hooks);
                hooks = install();
            }
        }
        KillTimer(null_mut(), timer);
        uninstall(hooks);
    }
}

/// SAFETY: call on the hook thread.
unsafe fn install() -> (HHOOK, HHOOK) {
    // SAFETY: as above; the procedures live for the whole program.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        (
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0),
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0),
        )
    }
}

/// SAFETY: call on the hook thread, with hooks it installed (or null).
unsafe fn uninstall((keyboard, mouse): (HHOOK, HHOOK)) {
    for hook in [keyboard, mouse] {
        if !hook.is_null() {
            // SAFETY: as above.
            unsafe { UnhookWindowsHookEx(hook) };
        }
    }
}

fn send(event: Event) {
    EVENTS.with(|e| {
        if let Ok(events) = e.try_borrow() {
            if let Some(events) = events.as_ref() {
                let _ = events.send(event);
            }
        }
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if kb.dwExtraInfo != INJECTED_TAG {
            send(Event::Key {
                vk: kb.vkCode,
                scan: kb.scanCode,
                modifiers: Modifiers::now(),
                // SAFETY: no preconditions.
                window: unsafe { GetForegroundWindow() } as isize,
            });
        }
    }
    // SAFETY: passing the event on unchanged, as Windows requires.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(
            wparam as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        send(Event::Click);
    }
    // SAFETY: as above.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn run_worker(inbox: mpsc::Receiver<Event>, mut expansions: Vec<Expansion>) {
    let mut typing = Typing::default();
    // The last window typed in, and whether it's the launcher's own.
    let mut last_window: Option<(isize, bool)> = None;
    // The window the last key was typed in (where an expansion goes).
    let mut key_window: isize = 0;
    while let Ok(event) = inbox.recv() {
        let input = match event {
            Event::Stop => break,
            Event::Update(fresh) => {
                expansions = fresh;
                typing.reset();
                continue;
            }
            Event::Click => Input::Click,
            Event::Key {
                vk,
                scan,
                modifiers,
                window,
            } => {
                key_window = window;
                let ours = match last_window {
                    Some((w, ours)) if w == window => ours,
                    _ => {
                        let ours = is_launcher(window);
                        last_window = Some((window, ours));
                        ours
                    }
                };
                if ours {
                    typing.reset();
                    continue;
                }
                Input::Key {
                    vk: vk as u16,
                    typed: translate(vk, scan, modifiers, window),
                    ctrl: modifiers.ctrl,
                    alt: modifiers.alt,
                    win: modifiers.win,
                    window,
                }
            }
        };
        if let Some(found) = typing.feed(input, &expansions) {
            expand_keyword(found, key_window);
        }
    }
}

fn is_launcher(window: isize) -> bool {
    let mut pid = 0u32;
    // SAFETY: a read-only query; a closed window just gives pid 0.
    unsafe { GetWindowThreadProcessId(window as HWND, &mut pid) };
    pid == std::process::id()
}

/// What a key types in `window`, with its keyboard layout.
fn translate(vk: u32, scan: u32, modifiers: Modifiers, window: isize) -> Typed {
    // Characters sent as text (on-screen keyboard, some input methods)
    // carry the character in the scan code.
    if vk == u32::from(VK_PACKET) {
        return char::from_u32(scan).map_or(Typed::Nothing, |c| Typed::Text(c.to_string()));
    }
    let mut state = [0u8; 256];
    let set = |state: &mut [u8; 256], vk: u16, on: bool, bit: u8| {
        if on {
            state[usize::from(vk)] = bit;
        }
    };
    set(&mut state, VK_SHIFT, modifiers.shift, 0x80);
    set(&mut state, VK_CONTROL, modifiers.ctrl, 0x80);
    set(&mut state, VK_MENU, modifiers.alt, 0x80);
    set(&mut state, VK_CAPITAL, modifiers.caps_lock, 0x01);
    let mut buf = [0u16; 8];
    // SAFETY: the buffers outlive the calls; a stale window gives the
    // default layout.
    let written = unsafe {
        let thread = GetWindowThreadProcessId(window as HWND, null_mut());
        let layout = GetKeyboardLayout(thread);
        ToUnicodeEx(
            vk,
            scan,
            state.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            TO_UNICODE_KEEP_STATE,
            layout,
        )
    };
    match written {
        n if n > 0 => Typed::Text(String::from_utf16_lossy(&buf[..n as usize])),
        n if n < 0 => Typed::Dead,
        _ => Typed::Nothing,
    }
}

/// Replaces the keyword just typed in `window` with its snippet: Backspace
/// over it, paste the snippet through the clipboard, then put back
/// everything the clipboard held.
///
/// The keyword is erased first: saving the clipboard can wait on the app
/// that copied (Excel renders its formats on demand), and keys typed
/// meanwhile would be erased instead. Every key goes only to `window`; if
/// you switched away, nothing is pressed.
fn expand_keyword(found: Found, window: isize) {
    wait_for_modifiers_up();
    if !in_front(window) {
        tracing::debug!("snippet keyword typed, but its window isn't in front any more");
        return;
    }
    if !press_repeatedly(VK_BACK, found.erase) {
        tracing::warn!("could not erase a snippet keyword");
        return;
    }
    let saved = system::save();
    let clipboard = saved
        .as_ref()
        .and_then(system::Saved::text)
        .unwrap_or_default();
    let text = expand::expand(
        &found.expansion.text,
        &clipboard,
        chrono::Local::now().naive_local(),
    );
    let written = match system::write(&Content::Text(text), true) {
        Ok(sequence) => sequence,
        Err(e) => {
            tracing::warn!("could not put a snippet on the clipboard: {e}");
            return;
        }
    };
    if in_front(window) {
        if !press_together(&[VK_CONTROL, VK_V]) {
            tracing::warn!("could not paste a snippet");
        }
        std::thread::sleep(RESTORE_DELAY);
    } else {
        tracing::debug!("the window changed before the snippet could be pasted");
    }
    // Only if nothing new was copied meanwhile.
    if system::sequence() == written {
        if let Some(saved) = &saved {
            if let Err(e) = system::put_back(saved) {
                tracing::debug!("could not put the clipboard back: {e}");
            }
        }
    }
}

fn in_front(window: isize) -> bool {
    // SAFETY: no preconditions.
    unsafe { GetForegroundWindow() as isize == window }
}

fn wait_for_modifiers_up() {
    let started = Instant::now();
    loop {
        let m = Modifiers::now();
        if !(m.shift || m.ctrl || m.alt || m.win) || started.elapsed() > MODIFIER_WAIT {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: INJECTED_TAG,
            },
        },
    }
}

fn send_input(inputs: &[INPUT]) -> bool {
    // SAFETY: `inputs` is a valid slice for the duration of the call.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    sent as usize == inputs.len()
}

/// Taps `vk` `times` times.
fn press_repeatedly(vk: u16, times: usize) -> bool {
    let inputs: Vec<INPUT> = (0..times)
        .flat_map(|_| [key(vk, false), key(vk, true)])
        .collect();
    send_input(&inputs)
}

/// Presses `keys` as one shortcut: down in order, up in reverse.
fn press_together(keys: &[u16]) -> bool {
    let mut inputs: Vec<INPUT> = keys.iter().map(|&k| key(k, false)).collect();
    inputs.extend(keys.iter().rev().map(|&k| key(k, true)));
    send_input(&inputs)
}
