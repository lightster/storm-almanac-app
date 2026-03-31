use crate::input_recorder::RawInputEvent;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::SystemTime;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VSC_TO_VK_EX};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW,
    RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLOSE, WM_INPUT, WNDCLASSW,
};

// Raw input keyboard flags
const RI_KEY_BREAK: u16 = 1;
const RI_KEY_E0: u16 = 2;

// Raw input mouse button flags
const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;
const RI_MOUSE_MIDDLE_BUTTON_UP: u16 = 0x0020;
const RI_MOUSE_BUTTON_4_DOWN: u16 = 0x0040;
const RI_MOUSE_BUTTON_4_UP: u16 = 0x0080;
const RI_MOUSE_BUTTON_5_DOWN: u16 = 0x0100;
const RI_MOUSE_BUTTON_5_UP: u16 = 0x0200;

// RAWINPUTHEADER dwType values
const RIM_TYPEMOUSE: u32 = 0;
const RIM_TYPEKEYBOARD: u32 = 1;

thread_local! {
    static LISTENER_TX: RefCell<Option<Sender<RawInputEvent>>> = RefCell::new(None);
    static LISTENER_RECORDING: RefCell<Option<Arc<AtomicBool>>> = RefCell::new(None);
}

/// Thread-safe wrapper around HWND for stopping the listener from another thread.
pub struct SendHwnd(HWND);
unsafe impl Send for SendHwnd {}

impl SendHwnd {
    pub fn close(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.0), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        process_raw_input(lparam);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn process_raw_input(lparam: LPARAM) {
    let recording = LISTENER_RECORDING.with(|r| r.borrow().clone());
    let Some(recording) = recording else { return };
    if !recording.load(Ordering::Relaxed) {
        return;
    }

    let hrawinput = HRAWINPUT(lparam.0 as _);
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;

    GetRawInputData(hrawinput, RID_INPUT, None, &mut size, header_size);
    if size == 0 {
        return;
    }

    let mut buffer = vec![0u8; size as usize];
    let copied = GetRawInputData(
        hrawinput,
        RID_INPUT,
        Some(buffer.as_mut_ptr() as *mut _),
        &mut size,
        header_size,
    );
    if copied == u32::MAX {
        return;
    }

    let raw = &*(buffer.as_ptr() as *const RAWINPUT);
    let now = SystemTime::now();

    let events = match raw.header.dwType {
        RIM_TYPEMOUSE => process_mouse(raw, now),
        RIM_TYPEKEYBOARD => process_keyboard(raw, now),
        _ => vec![],
    };

    LISTENER_TX.with(|tx| {
        if let Some(ref sender) = *tx.borrow() {
            for event in events {
                let _ = sender.send(event);
            }
        }
    });
}

unsafe fn process_keyboard(raw: &RAWINPUT, now: SystemTime) -> Vec<RawInputEvent> {
    let kb = raw.data.keyboard;
    let vk = kb.VKey;
    let scan = kb.MakeCode;
    let flags = kb.Flags;

    let is_up = (flags & RI_KEY_BREAK) != 0;
    let event_type: &'static str = if is_up { "ku" } else { "kd" };

    let resolved_vk = resolve_vk(vk, scan, flags);
    let name = vk_to_key_name(resolved_vk);
    let raw_code = if name.starts_with("Unknown(") {
        resolved_vk as u32
    } else {
        0
    };

    vec![RawInputEvent {
        time: now,
        event_type,
        key: name,
        raw: raw_code,
        name: None,
    }]
}

unsafe fn process_mouse(raw: &RAWINPUT, now: SystemTime) -> Vec<RawInputEvent> {
    let button_flags = raw.data.mouse.Anonymous.Anonymous.usButtonFlags;
    let mut events = Vec::new();

    const CHECKS: &[(u16, &str, &str, u32)] = &[
        (RI_MOUSE_LEFT_BUTTON_DOWN, "bd", "Left", 1),
        (RI_MOUSE_LEFT_BUTTON_UP, "bu", "Left", 1),
        (RI_MOUSE_RIGHT_BUTTON_DOWN, "bd", "Right", 2),
        (RI_MOUSE_RIGHT_BUTTON_UP, "bu", "Right", 2),
        (RI_MOUSE_MIDDLE_BUTTON_DOWN, "bd", "Middle", 3),
        (RI_MOUSE_MIDDLE_BUTTON_UP, "bu", "Middle", 3),
        (RI_MOUSE_BUTTON_4_DOWN, "bd", "Unknown(4)", 4),
        (RI_MOUSE_BUTTON_4_UP, "bu", "Unknown(4)", 4),
        (RI_MOUSE_BUTTON_5_DOWN, "bd", "Unknown(5)", 5),
        (RI_MOUSE_BUTTON_5_UP, "bu", "Unknown(5)", 5),
    ];

    for &(flag, etype, key, raw) in CHECKS {
        if button_flags & flag != 0 {
            events.push(RawInputEvent {
                time: now,
                event_type: etype,
                key: key.to_string(),
                raw,
                name: None,
            });
        }
    }

    events
}

fn resolve_vk(vk: u16, scan_code: u16, flags: u16) -> u16 {
    match vk {
        0x10 => {
            // VK_SHIFT: use scan code to disambiguate left/right
            let resolved = unsafe { MapVirtualKeyW(scan_code as u32, MAPVK_VSC_TO_VK_EX) };
            if resolved != 0 {
                resolved as u16
            } else {
                vk
            }
        }
        0x11 => {
            // VK_CONTROL: E0 flag means right
            if flags & RI_KEY_E0 != 0 {
                163 // VK_RCONTROL
            } else {
                162 // VK_LCONTROL
            }
        }
        0x12 => {
            // VK_MENU (Alt): E0 flag means right
            if flags & RI_KEY_E0 != 0 {
                165 // VK_RMENU
            } else {
                164 // VK_LMENU
            }
        }
        _ => vk,
    }
}

pub fn start_listener(
    tx: Sender<RawInputEvent>,
    recording: Arc<AtomicBool>,
) -> (std::thread::JoinHandle<()>, SendHwnd) {
    let (hwnd_tx, hwnd_rx) = std::sync::mpsc::channel::<SendHwnd>();

    let handle = std::thread::spawn(move || {
        LISTENER_TX.with(|t| *t.borrow_mut() = Some(tx));
        LISTENER_RECORDING.with(|r| *r.borrow_mut() = Some(recording));

        unsafe {
            let hinstance = GetModuleHandleW(None).unwrap();
            let class_name = w!("StormAlmanacRawInput");

            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            )
            .expect("Failed to create message-only window for raw input");

            let devices = [
                RAWINPUTDEVICE {
                    usUsagePage: 0x01, // HID_USAGE_PAGE_GENERIC
                    usUsage: 0x06,     // HID_USAGE_GENERIC_KEYBOARD
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: hwnd,
                },
                RAWINPUTDEVICE {
                    usUsagePage: 0x01,
                    usUsage: 0x02, // HID_USAGE_GENERIC_MOUSE
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: hwnd,
                },
            ];
            RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32)
                .expect("Failed to register raw input devices");

            let _ = hwnd_tx.send(SendHwnd(hwnd));

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_CLOSE {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        LISTENER_TX.with(|t| *t.borrow_mut() = None);
        LISTENER_RECORDING.with(|r| *r.borrow_mut() = None);
    });

    let hwnd = hwnd_rx
        .recv()
        .expect("Failed to receive HWND from listener thread");
    (handle, hwnd)
}

/// Maps a Windows virtual key code (after left/right disambiguation) to an
/// rdev-compatible key name string. Based on rdev's src/windows/keycodes.rs.
fn vk_to_key_name(vk: u16) -> String {
    match vk {
        164 => "Alt",
        165 => "AltGr",
        0x08 => "Backspace",
        20 => "CapsLock",
        162 => "ControlLeft",
        163 => "ControlRight",
        46 => "Delete",
        40 => "DownArrow",
        35 => "End",
        27 => "Escape",
        112 => "F1",
        113 => "F2",
        114 => "F3",
        115 => "F4",
        116 => "F5",
        117 => "F6",
        118 => "F7",
        119 => "F8",
        120 => "F9",
        121 => "F10",
        122 => "F11",
        123 => "F12",
        36 => "Home",
        37 => "LeftArrow",
        91 => "MetaLeft",
        92 => "MetaRight",
        34 => "PageDown",
        33 => "PageUp",
        0x0D => "Return",
        39 => "RightArrow",
        160 => "ShiftLeft",
        161 => "ShiftRight",
        32 => "Space",
        0x09 => "Tab",
        38 => "UpArrow",
        44 => "PrintScreen",
        145 => "ScrollLock",
        19 => "Pause",
        144 => "NumLock",
        192 => "BackQuote",
        49 => "Num1",
        50 => "Num2",
        51 => "Num3",
        52 => "Num4",
        53 => "Num5",
        54 => "Num6",
        55 => "Num7",
        56 => "Num8",
        57 => "Num9",
        48 => "Num0",
        189 => "Minus",
        187 => "Equal",
        81 => "KeyQ",
        87 => "KeyW",
        69 => "KeyE",
        82 => "KeyR",
        84 => "KeyT",
        89 => "KeyY",
        85 => "KeyU",
        73 => "KeyI",
        79 => "KeyO",
        80 => "KeyP",
        219 => "LeftBracket",
        221 => "RightBracket",
        65 => "KeyA",
        83 => "KeyS",
        68 => "KeyD",
        70 => "KeyF",
        71 => "KeyG",
        72 => "KeyH",
        74 => "KeyJ",
        75 => "KeyK",
        76 => "KeyL",
        186 => "SemiColon",
        222 => "Quote",
        220 => "BackSlash",
        226 => "IntlBackslash",
        90 => "KeyZ",
        88 => "KeyX",
        67 => "KeyC",
        86 => "KeyV",
        66 => "KeyB",
        78 => "KeyN",
        77 => "KeyM",
        188 => "Comma",
        190 => "Dot",
        191 => "Slash",
        45 => "Insert",
        109 => "KpMinus",
        107 => "KpPlus",
        106 => "KpMultiply",
        111 => "KpDivide",
        96 => "Kp0",
        97 => "Kp1",
        98 => "Kp2",
        99 => "Kp3",
        100 => "Kp4",
        101 => "Kp5",
        102 => "Kp6",
        103 => "Kp7",
        104 => "Kp8",
        105 => "Kp9",
        110 => "KpDelete",
        _ => return format!("Unknown({})", vk),
    }
    .to_string()
}
