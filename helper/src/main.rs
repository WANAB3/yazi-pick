//! yazi-pick-ax: native helper for yazi-pick.
//!
//! Talks to the Accessibility API in-process instead of going through
//! System Events (Apple Events IPC, ~50-150ms per query), which makes the
//! post-pick dialog automation several times faster than osascript.
//!
//! Subcommands:
//!   frontmost         print "<pid>\t<name>\t<focused-window-title>"
//!   push <pid> <path> [dialog-title]
//!                     type <path> into the file dialog owned by <pid>
//!
//! The push logic mirrors the osascript fallback in the yazi-pick script:
//!   1. re-activate the app and wait until it is frontmost
//!   2. if the window server sees more windows than the AX tree exposes,
//!      an AX-invisible dialog (Gecko remote panel) exists -> blind keystrokes
//!   3. re-raise the dialog window by title: reactivating the app often hands
//!      key status back to the document window instead of the dialog panel
//!   4. then open the Go-to-Folder sheet and write the path via AX,
//!      accepting only a newly focused text field inside an AXSheet
//!   5. if no such field appears, press ESC and fail without typing anything

#![allow(non_upper_case_globals)]

use std::process::exit;
use std::thread::sleep;
use std::time::{Duration, Instant};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::window::{copy_window_info, kCGNullWindowID, kCGWindowListOptionOnScreenOnly};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

type Id = *mut Object;
type AXUIElementRef = CFTypeRef;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
    fn AXIsProcessTrusted() -> bool;
    fn AXValueCreate(value_type: u32, value_ptr: *const std::ffi::c_void) -> CFTypeRef;
    fn AXValueGetValue(
        value: CFTypeRef,
        value_type: u32,
        value_ptr: *mut std::ffi::c_void,
    ) -> bool;
}

// Make sure AppKit is loaded so class!(NSWorkspace) etc. resolve.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

const kAXValueCGPointType: u32 = 1;
const kAXValueCGSizeType: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPointRaw {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSizeRaw {
    width: f64,
    height: f64,
}

const KEY_A: u16 = 0;
const KEY_G: u16 = 5;
const KEY_V: u16 = 9;
const KEY_RETURN: u16 = 36;
const KEY_ESC: u16 = 53;

fn die(msg: &str) -> ! {
    eprintln!("yazi-pick-ax: {msg}");
    exit(1);
}

// ── AX plumbing ────────────────────────────────────────────────

fn ax_copy(el: AXUIElementRef, attr: &str) -> Option<CFType> {
    let attr = CFString::new(attr);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(el, attr.as_concrete_TypeRef(), &mut out) };
    if err == 0 && !out.is_null() {
        Some(unsafe { CFType::wrap_under_create_rule(out) })
    } else {
        None
    }
}

fn ax_role(el: &CFType) -> String {
    ax_copy(el.as_CFTypeRef(), "AXRole")
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn ax_title(el: AXUIElementRef) -> String {
    ax_copy(el, "AXTitle")
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn is_text_input(el: &CFType) -> bool {
    matches!(ax_role(el).as_str(), "AXTextField" | "AXComboBox")
}

/// The Go-to-Folder field lives inside an AXSheet (AXTextField <- AXSheet <- AXWindow).
fn in_sheet(el: &CFType) -> bool {
    let mut cur = el.clone();
    for _ in 0..12 {
        let Some(parent) = ax_copy(cur.as_CFTypeRef(), "AXParent") else {
            return false;
        };
        match ax_role(&parent).as_str() {
            "AXSheet" => return true,
            "AXWindow" | "AXApplication" => return false,
            _ => cur = parent,
        }
    }
    false
}

fn focused_element(app: AXUIElementRef) -> Option<CFType> {
    ax_copy(app, "AXFocusedUIElement")
}

/// Title of the app's focused window ("" if none). push re-raises this window
/// by title later, because reactivating the app may key a different window.
fn focused_window_title(pid: i32) -> String {
    let app = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) };
    ax_copy(app.as_CFTypeRef(), "AXFocusedWindow")
        .map(|w| ax_title(w.as_CFTypeRef()))
        .unwrap_or_default()
}

fn ax_window_count(app: AXUIElementRef) -> usize {
    ax_copy(app, "AXWindows")
        .and_then(|v| v.downcast::<CFArray>())
        .map(|a| a.len() as usize)
        .unwrap_or(0)
}

// ── Window server ──────────────────────────────────────────────

/// Windows of `pid` as seen by the window server (onscreen, reasonably sized).
fn cg_window_count(pid: i32) -> usize {
    let Some(info) = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID) else {
        return 0;
    };
    let mut n = 0;
    for item in info.iter() {
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*item as *const _) };
        let num = |key: &str| -> f64 {
            dict.find(CFString::new(key))
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_f64())
                .unwrap_or(0.0)
        };
        if num("kCGWindowOwnerPID") as i32 != pid || num("kCGWindowAlpha") <= 0.0 {
            continue;
        }
        let bounds = dict
            .find(CFString::new("kCGWindowBounds"))
            .and_then(|v| v.downcast::<CFDictionary>());
        let Some(bounds) = bounds else { continue };
        let bounds: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(bounds.as_concrete_TypeRef()) };
        let dim = |key: &str| -> f64 {
            bounds
                .find(CFString::new(key))
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_f64())
                .unwrap_or(0.0)
        };
        if dim("Width") >= 300.0 && dim("Height") >= 150.0 {
            n += 1;
        }
    }
    n
}

// ── AppKit (NSWorkspace / NSRunningApplication / NSPasteboard) ─

fn frontmost_app() -> (i32, String) {
    unsafe {
        let ws: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: Id = msg_send![ws, frontmostApplication];
        if app.is_null() {
            die("no frontmost application");
        }
        let pid: i32 = msg_send![app, processIdentifier];
        let name: Id = msg_send![app, localizedName];
        let name = if name.is_null() {
            String::new()
        } else {
            CFString::wrap_under_get_rule(name as CFStringRef).to_string()
        };
        (pid, name)
    }
}

fn activate(pid: i32) -> bool {
    unsafe {
        let app: Id =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if app.is_null() {
            return false;
        }
        // NSApplicationActivateIgnoringOtherApps
        let _: bool = msg_send![app, activateWithOptions: 2u64];
        true
    }
}

fn pasteboard_get() -> Option<String> {
    unsafe {
        let pb: Id = msg_send![class!(NSPasteboard), generalPasteboard];
        let ty = CFString::new("public.utf8-plain-text");
        let s: Id = msg_send![pb, stringForType: ty.as_concrete_TypeRef() as Id];
        if s.is_null() {
            None
        } else {
            Some(CFString::wrap_under_get_rule(s as CFStringRef).to_string())
        }
    }
}

fn pasteboard_set(text: &str) {
    unsafe {
        let pb: Id = msg_send![class!(NSPasteboard), generalPasteboard];
        let _: i64 = msg_send![pb, clearContents];
        let ty = CFString::new("public.utf8-plain-text");
        let val = CFString::new(text);
        let _: bool = msg_send![pb, setString: val.as_concrete_TypeRef() as Id
                                     forType: ty.as_concrete_TypeRef() as Id];
    }
}

// ── Keyboard ───────────────────────────────────────────────────

fn key(code: u16, flags: CGEventFlags) {
    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .unwrap_or_else(|_| die("cannot create event source"));
    for down in [true, false] {
        let ev = CGEvent::new_keyboard_event(src.clone(), code, down)
            .unwrap_or_else(|_| die("cannot create keyboard event"));
        ev.set_flags(flags);
        ev.post(CGEventTapLocation::HID);
        sleep(Duration::from_millis(12));
    }
}

fn ms(n: u64) {
    sleep(Duration::from_millis(n));
}

// ── push ───────────────────────────────────────────────────────

/// Pid of the app owning keyboard focus, queried live from the AX server.
/// (NSWorkspace's frontmostApplication is cached per-process and never
/// updates in a run-loop-less CLI, so it cannot be polled.)
fn focused_app_pid() -> i32 {
    let sys = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
    ax_copy(sys.as_CFTypeRef(), "AXFocusedApplication")
        .map(|app| {
            let mut pid: i32 = 0;
            unsafe { AXUIElementGetPid(app.as_CFTypeRef(), &mut pid) };
            pid
        })
        .unwrap_or(0)
}

fn wait_frontmost(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_activate: Option<Instant> = None;
    loop {
        // Re-request activation sparingly; hammering the window server can
        // keep the activation hand-off from ever settling.
        if last_activate.map_or(true, |t| t.elapsed() >= Duration::from_millis(400)) {
            activate(pid);
            last_activate = Some(Instant::now());
        }
        if focused_app_pid() == pid {
            return;
        }
        if Instant::now() > deadline {
            die("target app did not come to front");
        }
        ms(20);
    }
}

/// Center the app's focused window horizontally, Spotlight-style (upper third).
fn center(pid: i32) {
    let app = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) };
    let deadline = Instant::now() + Duration::from_secs(3);
    let win = loop {
        if let Some(w) = ax_copy(app.as_CFTypeRef(), "AXFocusedWindow") {
            break w;
        }
        if Instant::now() > deadline {
            die("no focused window to center");
        }
        ms(20);
    };
    let mut size = CGSizeRaw::default();
    let got = ax_copy(win.as_CFTypeRef(), "AXSize")
        .map(|v| unsafe {
            AXValueGetValue(
                v.as_CFTypeRef(),
                kAXValueCGSizeType,
                &mut size as *mut _ as *mut std::ffi::c_void,
            )
        })
        .unwrap_or(false);
    if !got {
        die("cannot read window size");
    }
    let screen = core_graphics::display::CGDisplay::main().bounds();
    let pos = CGPointRaw {
        x: screen.origin.x + (screen.size.width - size.width) / 2.0,
        y: screen.origin.y + (screen.size.height - size.height) * 0.22,
    };
    let value = unsafe {
        CFType::wrap_under_create_rule(AXValueCreate(
            kAXValueCGPointType,
            &pos as *const _ as *const std::ffi::c_void,
        ))
    };
    let err = unsafe {
        AXUIElementSetAttributeValue(
            win.as_CFTypeRef(),
            CFString::new("AXPosition").as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    };
    if err != 0 {
        die("cannot move window");
    }
}

fn blind_input(path: &str) {
    let saved = pasteboard_get();
    pasteboard_set(path);
    let cmd_shift = CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagShift;
    key(KEY_G, cmd_shift); // Go-to-Folder overlay
    ms(500);
    key(KEY_A, CGEventFlags::CGEventFlagCommand);
    ms(100);
    key(KEY_V, CGEventFlags::CGEventFlagCommand);
    ms(300);
    key(KEY_RETURN, CGEventFlags::empty()); // Go
    ms(600);
    key(KEY_RETURN, CGEventFlags::empty()); // confirm "Open"
    ms(200);
    if let Some(saved) = saved {
        pasteboard_set(&saved);
    }
}

/// Bring the window with the given title back to key status. Reactivating an
/// app after the picker often keys the document window instead of the dialog
/// panel, and Cmd+Shift+G would then go to the wrong window.
fn raise_window(app: AXUIElementRef, title: &str) {
    let Some(wins) = ax_copy(app, "AXWindows").and_then(|v| v.downcast::<CFArray>()) else {
        return;
    };
    for w in wins.iter() {
        let w = *w as AXUIElementRef;
        if ax_title(w) == title {
            unsafe { AXUIElementPerformAction(w, CFString::new("AXRaise").as_concrete_TypeRef()) };
            break;
        }
    }
    // Wait until the raise actually lands (bounded; proceed regardless).
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        let focused = ax_copy(app, "AXFocusedWindow")
            .map_or(false, |w| ax_title(w.as_CFTypeRef()) == title);
        if focused {
            return;
        }
        ms(20);
    }
}

fn push(pid: i32, path: &str, dialog_title: Option<&str>) {
    if !unsafe { AXIsProcessTrusted() } {
        die("accessibility permission missing for this process");
    }

    wait_frontmost(pid);
    ms(150);

    let app = unsafe { AXUIElementCreateApplication(pid) };
    let app = unsafe { CFType::wrap_under_create_rule(app) };
    let app_ref = app.as_CFTypeRef();

    // Blind mode only when an AX-invisible dialog (Gecko remote panel) exists.
    let ax_count = ax_window_count(app_ref);
    let cg_count = cg_window_count(pid);
    if cg_count > ax_count {
        eprintln!("yazi-pick-ax: AX-invisible dialog (cg={cg_count} ax={ax_count}): blind input");
        blind_input(path);
        return;
    }

    // Reactivation may have keyed the document window instead of the dialog;
    // re-raise the window that was focused when the pick started.
    if let Some(title) = dialog_title.filter(|t| !t.is_empty()) {
        raise_window(app_ref, title);
    }

    // Save dialogs focus the filename field from the start; remember it so we
    // can tell it apart from the Go-to-Folder sheet's field.
    let pre_field = focused_element(app_ref).filter(is_text_input_ref);

    let cmd_shift = CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagShift;
    key(KEY_G, cmd_shift);

    // Wait for a newly focused text field inside an AXSheet.
    let mut go_field: Option<CFType> = None;
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        if let Some(fe) = focused_element(app_ref) {
            if is_text_input(&fe)
                && in_sheet(&fe)
                && pre_field.as_ref().map_or(true, |p| !fe.eq(p))
            {
                go_field = Some(fe);
                break;
            }
        }
        ms(20);
    }

    let Some(field) = go_field else {
        key(KEY_ESC, CGEventFlags::empty()); // close whatever Cmd+Shift+G opened
        die("could not find the Go-to-Folder sheet (no dialog open, or unsupported app)");
    };

    // Write the path via AX (replaces the whole value; immune to IME and
    // leftover text) and verify by reading it back.
    let value = CFString::new(path);
    let err = unsafe {
        AXUIElementSetAttributeValue(
            field.as_CFTypeRef(),
            CFString::new("AXValue").as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    };
    if err != 0 {
        die("failed to write the path into the field");
    }
    ms(60);
    let back = ax_copy(field.as_CFTypeRef(), "AXValue")
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string())
        .unwrap_or_default();
    if back != path {
        die("path verification failed after writing");
    }

    key(KEY_RETURN, CGEventFlags::empty()); // Go (leaves the file selected)

    // Wait for the sheet to close and focus to return to the file list.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match focused_element(app_ref) {
            Some(fe) if is_text_input(&fe) => ms(20),
            _ => break,
        }
    }
    ms(150);
    key(KEY_RETURN, CGEventFlags::empty()); // confirm "Open"
}

fn is_text_input_ref(el: &CFType) -> bool {
    is_text_input(el)
}

// ── main ───────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("frontmost") => {
            let (pid, name) = frontmost_app();
            println!("{pid}\t{name}\t{}", focused_window_title(pid));
        }
        Some("activate") if args.len() == 3 => {
            let pid: i32 = args[2].parse().unwrap_or_else(|_| die("invalid pid"));
            wait_frontmost(pid);
        }
        Some("center") if args.len() == 3 => {
            let pid: i32 = args[2].parse().unwrap_or_else(|_| die("invalid pid"));
            center(pid);
        }
        Some("push") if args.len() == 4 || args.len() == 5 => {
            let pid: i32 = args[2].parse().unwrap_or_else(|_| die("invalid pid"));
            push(pid, &args[3], args.get(4).map(String::as_str));
        }
        _ => {
            eprintln!(
                "usage: yazi-pick-ax frontmost | activate <pid> | center <pid> | push <pid> <path> [dialog-title]"
            );
            exit(2);
        }
    }
}
