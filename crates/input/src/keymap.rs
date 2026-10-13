//! Physical key → native keycode, per platform.
//!
//! Keys travel as W3C `KeyboardEvent.code` values, which name a *position* on the keyboard
//! (US layout naming): "KeyQ" is the key left of "KeyW" whatever letter it prints. The host
//! replays that position, and its own layout decides the character, like a real keyboard.

/// Native codes for one physical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCodes {
    /// macOS virtual keycode (`kVK_*`).
    pub mac: Option<u16>,
    /// Windows set-1 scancode; `0xE0xx` marks an extended key.
    pub win: Option<u16>,
    /// Linux evdev code (`KEY_*`); X11 keycode = evdev + 8.
    pub evdev: Option<u16>,
}

const fn k(mac: u16, win: u16, evdev: u16) -> KeyCodes {
    KeyCodes {
        mac: Some(mac),
        win: Some(win),
        evdev: Some(evdev),
    }
}

const fn no_mac(win: u16, evdev: u16) -> KeyCodes {
    KeyCodes {
        mac: None,
        win: Some(win),
        evdev: Some(evdev),
    }
}

#[rustfmt::skip]
const TABLE: &[(&str, KeyCodes)] = &[
    ("KeyA", k(0x00, 0x1E, 30)), ("KeyB", k(0x0B, 0x30, 48)), ("KeyC", k(0x08, 0x2E, 46)),
    ("KeyD", k(0x02, 0x20, 32)), ("KeyE", k(0x0E, 0x12, 18)), ("KeyF", k(0x03, 0x21, 33)),
    ("KeyG", k(0x05, 0x22, 34)), ("KeyH", k(0x04, 0x23, 35)), ("KeyI", k(0x22, 0x17, 23)),
    ("KeyJ", k(0x26, 0x24, 36)), ("KeyK", k(0x28, 0x25, 37)), ("KeyL", k(0x25, 0x26, 38)),
    ("KeyM", k(0x2E, 0x32, 50)), ("KeyN", k(0x2D, 0x31, 49)), ("KeyO", k(0x1F, 0x18, 24)),
    ("KeyP", k(0x23, 0x19, 25)), ("KeyQ", k(0x0C, 0x10, 16)), ("KeyR", k(0x0F, 0x13, 19)),
    ("KeyS", k(0x01, 0x1F, 31)), ("KeyT", k(0x11, 0x14, 20)), ("KeyU", k(0x20, 0x16, 22)),
    ("KeyV", k(0x09, 0x2F, 47)), ("KeyW", k(0x0D, 0x11, 17)), ("KeyX", k(0x07, 0x2D, 45)),
    ("KeyY", k(0x10, 0x15, 21)), ("KeyZ", k(0x06, 0x2C, 44)),

    ("Digit1", k(0x12, 0x02, 2)), ("Digit2", k(0x13, 0x03, 3)), ("Digit3", k(0x14, 0x04, 4)),
    ("Digit4", k(0x15, 0x05, 5)), ("Digit5", k(0x17, 0x06, 6)), ("Digit6", k(0x16, 0x07, 7)),
    ("Digit7", k(0x1A, 0x08, 8)), ("Digit8", k(0x1C, 0x09, 9)), ("Digit9", k(0x19, 0x0A, 10)),
    ("Digit0", k(0x1D, 0x0B, 11)),

    ("Minus", k(0x1B, 0x0C, 12)), ("Equal", k(0x18, 0x0D, 13)),
    ("BracketLeft", k(0x21, 0x1A, 26)), ("BracketRight", k(0x1E, 0x1B, 27)),
    ("Backslash", k(0x2A, 0x2B, 43)), ("Semicolon", k(0x29, 0x27, 39)),
    ("Quote", k(0x27, 0x28, 40)), ("Backquote", k(0x32, 0x29, 41)),
    ("Comma", k(0x2B, 0x33, 51)), ("Period", k(0x2F, 0x34, 52)), ("Slash", k(0x2C, 0x35, 53)),
    ("IntlBackslash", k(0x0A, 0x56, 86)),

    ("Enter", k(0x24, 0x1C, 28)), ("Tab", k(0x30, 0x0F, 15)), ("Space", k(0x31, 0x39, 57)),
    ("Backspace", k(0x33, 0x0E, 14)), ("Escape", k(0x35, 0x01, 1)), ("CapsLock", k(0x39, 0x3A, 58)),

    ("ShiftLeft", k(0x38, 0x2A, 42)), ("ShiftRight", k(0x3C, 0x36, 54)),
    ("ControlLeft", k(0x3B, 0x1D, 29)), ("ControlRight", k(0x3E, 0xE01D, 97)),
    ("AltLeft", k(0x3A, 0x38, 56)), ("AltRight", k(0x3D, 0xE038, 100)),
    ("MetaLeft", k(0x37, 0xE05B, 125)), ("MetaRight", k(0x36, 0xE05C, 126)),
    ("ContextMenu", k(0x6E, 0xE05D, 127)),

    ("ArrowLeft", k(0x7B, 0xE04B, 105)), ("ArrowRight", k(0x7C, 0xE04D, 106)),
    ("ArrowUp", k(0x7E, 0xE048, 103)), ("ArrowDown", k(0x7D, 0xE050, 108)),
    ("Insert", k(0x72, 0xE052, 110)), ("Delete", k(0x75, 0xE053, 111)),
    ("Home", k(0x73, 0xE047, 102)), ("End", k(0x77, 0xE04F, 107)),
    ("PageUp", k(0x74, 0xE049, 104)), ("PageDown", k(0x79, 0xE051, 109)),
    ("PrintScreen", no_mac(0xE037, 99)), ("ScrollLock", no_mac(0x46, 70)),

    ("F1", k(0x7A, 0x3B, 59)), ("F2", k(0x78, 0x3C, 60)), ("F3", k(0x63, 0x3D, 61)),
    ("F4", k(0x76, 0x3E, 62)), ("F5", k(0x60, 0x3F, 63)), ("F6", k(0x61, 0x40, 64)),
    ("F7", k(0x62, 0x41, 65)), ("F8", k(0x64, 0x42, 66)), ("F9", k(0x65, 0x43, 67)),
    ("F10", k(0x6D, 0x44, 68)), ("F11", k(0x67, 0x57, 87)), ("F12", k(0x6F, 0x58, 88)),

    ("NumLock", k(0x47, 0xE045, 69)),
    ("Numpad0", k(0x52, 0x52, 82)), ("Numpad1", k(0x53, 0x4F, 79)), ("Numpad2", k(0x54, 0x50, 80)),
    ("Numpad3", k(0x55, 0x51, 81)), ("Numpad4", k(0x56, 0x4B, 75)), ("Numpad5", k(0x57, 0x4C, 76)),
    ("Numpad6", k(0x58, 0x4D, 77)), ("Numpad7", k(0x59, 0x47, 71)), ("Numpad8", k(0x5B, 0x48, 72)),
    ("Numpad9", k(0x5C, 0x49, 73)),
    ("NumpadDecimal", k(0x41, 0x53, 83)), ("NumpadMultiply", k(0x43, 0x37, 55)),
    ("NumpadAdd", k(0x45, 0x4E, 78)), ("NumpadSubtract", k(0x4E, 0x4A, 74)),
    ("NumpadDivide", k(0x4B, 0xE035, 98)), ("NumpadEnter", k(0x4C, 0xE01C, 96)),
    ("NumpadEqual", k(0x51, 0x59, 117)),
];

pub fn lookup(code: &str) -> Option<KeyCodes> {
    TABLE.iter().find(|(c, _)| *c == code).map(|(_, k)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn known_and_unknown_keys() {
        assert_eq!(lookup("KeyA").unwrap().win, Some(0x1E));
        assert_eq!(lookup("ArrowUp").unwrap().evdev, Some(103));
        assert_eq!(lookup("PrintScreen").unwrap().mac, None);
        assert!(lookup("NotAKey").is_none());
    }

    #[test]
    fn no_duplicate_codes_per_platform() {
        let mut seen = (
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
        );
        for (name, keys) in TABLE {
            assert!(seen.0.insert(*name), "duplicate name {name}");
            if let Some(m) = keys.mac {
                assert!(seen.1.insert(m), "duplicate mac code for {name}");
            }
            if let Some(w) = keys.win {
                assert!(seen.2.insert(w), "duplicate win code for {name}");
            }
            if let Some(e) = keys.evdev {
                assert!(seen.3.insert(e), "duplicate evdev code for {name}");
            }
        }
    }
}
