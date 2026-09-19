//! Windows hotkey vendor — a listen-only low-level keyboard hook that turns raw
//! **right**-Control events into [`Edge`]s and pumps the thread's message queue. The
//! OS trigger behind the attention gestures ([`crate::body::gesture`]), and the twin
//! of [`crate::foundation::vendors::macos_hotkey`].
//!
//! **Only the right Control key triggers the gestures**, for the reason the Mac binds
//! the right Command: the *left* one is the everyday shortcut modifier here — held
//! while reaching for the next key, rested on mid-thought deciding which shortcut to
//! press — so a hold/double-tap detector bound to it would fire on that noise. The
//! right Control is almost never chorded or rested on, which makes it a quiet,
//! dedicated trigger. Caps Lock was the other candidate and loses on one fact: it is a
//! *toggle*, so a gesture on it could only avoid turning caps on by **consuming** the
//! event — and consuming is what this hook must never do.
//!
//! ## Why a hook rather than `RegisterHotKey`
//!
//! `RegisterHotKey` reports one chord going down, consumes it, and says nothing about
//! release. All three gestures here are about *timing between edges* — a tap, two taps,
//! a press still held — so the release edge is not optional, and consuming the key
//! would take the person's own right-Control away. `WH_KEYBOARD_LL` sees both edges,
//! distinguishes left from right (unlike `WM_KEYDOWN`, which reports a bare
//! `VK_CONTROL`), and passes every event on untouched.
//!
//! ## What the callback may do
//!
//! Windows drops a low-level hook that does not return within
//! `HKEY_CURRENT_USER\Control Panel\Desktop\LowLevelHooksTimeout` (300 ms by default) —
//! silently, and the person's keyboard is what pays. So the callback does one
//! non-blocking send onto an unbounded channel and returns. All recognition — the
//! double-tap window, the hold's two thresholds — lives in
//! [`crate::body::capabilities::hotkey`] and [`crate::body::gesture`], driven on the
//! runtime against one clock.
//!
//! Nothing here needs a grant: Windows has no Input Monitoring prompt, and a
//! low-level hook installed by an ordinary process in the interactive session simply
//! works. The one thing it does not see is a window running **elevated** while this
//! process is not — those keystrokes never reach an unelevated hook, so a gesture made
//! while an admin window has focus is silently missed. That is UIPI, not a bug to fix.

use std::cell::{Cell, RefCell};

use anyhow::anyhow;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::body::capabilities::hotkey::Edge;

thread_local! {
    /// Where the hook callback finds the sink to emit into. A `WH_KEYBOARD_LL` hook is
    /// called on the thread that installed it, through that thread's message queue, so
    /// thread-local is exactly the right lifetime — and it keeps this off a global
    /// `Mutex` the realtime-ish callback would have to take.
    static SINK: RefCell<Option<Box<dyn Fn(Edge)>>> = const { RefCell::new(None) };

    /// Whether the attention key is currently down, so a held modifier's auto-repeat
    /// (Windows delivers a `WM_KEYDOWN` per repeat) stays *one* press rather than a
    /// stream of taps.
    static KEY_DOWN: Cell<bool> = const { Cell::new(false) };
}

/// Install the hook on the **current thread** and pump its messages, calling `on_edge`
/// for each raw right-Control edge. **Blocks forever** — a low-level hook only receives
/// events while its installing thread pumps — so call it from a thread dedicated to the
/// gesture. Returns `Err` only if the hook cannot be installed.
pub fn run(on_edge: impl Fn(Edge) + 'static) -> anyhow::Result<()> {
    SINK.with(|s| *s.borrow_mut() = Some(Box::new(on_edge)));

    // SAFETY: `WH_KEYBOARD_LL` takes its hook procedure from this process, so the
    // module handle is ignored and may be null; thread id 0 means global. The procedure
    // is a plain `extern "system" fn` with no captured state — everything it touches is
    // the thread-local above, on this very thread.
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), std::ptr::null_mut(), 0) };
    if hook.is_null() {
        SINK.with(|s| *s.borrow_mut() = None);
        return Err(anyhow!("could not install the low-level keyboard hook"));
    }

    // The pump. A low-level hook is delivered by having the OS post to this queue, so a
    // thread that stops pumping stops seeing keys — there is no message this loop
    // handles itself, and that is fine: pumping *is* the work. `GetMessageW` returns 0
    // on `WM_QUIT` and -1 on error; either ends the loop, which in practice only happens
    // at shutdown.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let got = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if got <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // Deliberately *not* unhooked: this returns only as the process is going away, and
    // `UnhookWindowsHookEx` here would be tidiness on a thread nothing waits for.
    Ok(())
}

/// The hook procedure. Emits at most one [`Edge`] per event and **always** passes the
/// event on with `CallNextHookEx` — this hook never consumes, so the person's own
/// right-Control, and every other key, behaves exactly as if it were not installed.
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Anything but HC_ACTION must be passed straight through without inspecting the
    // payload — that is the documented contract, not a shortcut.
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if let Some(edge) = classify(wparam as u32, info.vkCode) {
            SINK.with(|s| {
                if let Some(f) = s.borrow().as_ref() {
                    f(edge);
                }
            });
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Turn one keyboard message into the edge it means, or `None` for the events the
/// recognizers must not see. Pure, so the whole translation is testable on any host.
///
/// Three rules, and the third is the one that is easy to get wrong:
/// 1. the attention key's own down/up are `Down`/`Up`, with repeats collapsed;
/// 2. any *other* key going down is `Other`, which breaks a pending tap and disarms a
///    pending hold, so `Ctrl+C` is neither a glance nor an attention hold;
/// 3. **other modifiers are not `Other`.** macOS reports modifier transitions as
///    `FlagsChanged` and never as `KeyDown`, so pressing Shift there does not break a
///    gesture; Windows delivers `VK_LSHIFT` as an ordinary `WM_KEYDOWN`, and without
///    this filter the same physical act would break the gesture on one platform and not
///    the other. Holding the attention key *and* Shift is one gesture, either way.
fn classify(message: u32, vk_code: u32) -> Option<Edge> {
    let vk = vk_code as u16;
    let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
    let up = message == WM_KEYUP || message == WM_SYSKEYUP;

    if vk == VK_RCONTROL {
        return if down {
            // Auto-repeat while held: only the rising edge is a press.
            KEY_DOWN.with(|held| (!held.replace(true)).then_some(Edge::Down))
        } else if up {
            KEY_DOWN.with(|held| held.replace(false).then_some(Edge::Up))
        } else {
            None
        };
    }

    if down && !is_modifier(vk) {
        return Some(Edge::Other);
    }
    None
}

/// Whether this virtual-key is a modifier, and so exempt from rule 3 above.
fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_CONTROL
            | VK_LCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
            | VK_CAPITAL
    )
}

/// **These have never been run.** The module is Windows-only, so they can only execute
/// on a Windows host, and none of the machines this repo is developed from is one. What
/// *is* checked from here is that they still compile for the target: `make exe` runs
/// `cargo xwin check --tests` before it builds, so a rule below that stops typechecking
/// fails the build check rather than rotting until somebody finds a Windows box.
#[cfg(test)]
mod tests {
    use super::*;

    /// The thread-local repeat state is shared across these, so each test starts by
    /// putting the key back up.
    fn reset() {
        KEY_DOWN.with(|h| h.set(false));
    }

    #[test]
    fn the_attention_key_produces_one_press_however_long_it_repeats() {
        reset();
        assert_eq!(classify(WM_KEYDOWN, VK_RCONTROL as u32), Some(Edge::Down));
        assert_eq!(classify(WM_KEYDOWN, VK_RCONTROL as u32), None, "auto-repeat is still one press");
        assert_eq!(classify(WM_KEYDOWN, VK_RCONTROL as u32), None);
        assert_eq!(classify(WM_KEYUP, VK_RCONTROL as u32), Some(Edge::Up));
        assert_eq!(classify(WM_KEYUP, VK_RCONTROL as u32), None, "a second release is not an edge");
    }

    #[test]
    fn the_left_control_is_not_the_attention_key() {
        reset();
        // It is an ordinary key going down as far as the gesture is concerned — and not
        // even that, because it is a modifier.
        assert_eq!(classify(WM_KEYDOWN, VK_LCONTROL as u32), None);
        assert_eq!(classify(WM_KEYUP, VK_LCONTROL as u32), None);
    }

    #[test]
    fn a_chord_breaks_the_gesture() {
        reset();
        assert_eq!(classify(WM_KEYDOWN, VK_RCONTROL as u32), Some(Edge::Down));
        assert_eq!(classify(WM_KEYDOWN, b'C' as u32), Some(Edge::Other), "Ctrl+C is not a gesture");
        assert_eq!(classify(WM_KEYUP, b'C' as u32), None, "only key *down* breaks it");
    }

    #[test]
    fn another_modifier_does_not_break_the_gesture() {
        reset();
        assert_eq!(classify(WM_KEYDOWN, VK_RCONTROL as u32), Some(Edge::Down));
        for vk in [VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_LWIN, VK_CAPITAL] {
            assert_eq!(
                classify(WM_KEYDOWN, vk as u32),
                None,
                "modifiers arrive as FlagsChanged on macOS and must not break it here either"
            );
        }
        assert_eq!(classify(WM_KEYUP, VK_RCONTROL as u32), Some(Edge::Up));
    }

    #[test]
    fn a_system_key_down_counts_like_any_other() {
        reset();
        // Alt+F4 and friends arrive as WM_SYSKEYDOWN; they break a gesture the same way.
        assert_eq!(classify(WM_SYSKEYDOWN, b'F' as u32), Some(Edge::Other));
    }
}
