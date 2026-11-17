//! Reserved shortcuts (spec 05 §10, 01 §5.3): pages never receive them.
//! The tab view gives these keys to the menu bar before WebKit offers them
//! to the page. They are matched by physical key (`keyCode`), so they work
//! across keyboard layouts.

use std::cell::RefCell;
use std::collections::BTreeSet;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags};

thread_local! {
    /// (virtual key code, modifier bits) of every reserved shortcut.
    static RESERVED: RefCell<BTreeSet<(u16, usize)>> = const { RefCell::new(BTreeSet::new()) };
}

/// The macOS virtual key code of a shortcut's key (US positions).
pub fn key_code(key: &str) -> Option<u16> {
    const LETTERS: [(char, u16); 26] = [
        ('A', 0),
        ('S', 1),
        ('D', 2),
        ('F', 3),
        ('H', 4),
        ('G', 5),
        ('Z', 6),
        ('X', 7),
        ('C', 8),
        ('V', 9),
        ('B', 11),
        ('Q', 12),
        ('W', 13),
        ('E', 14),
        ('R', 15),
        ('Y', 16),
        ('T', 17),
        ('O', 31),
        ('U', 32),
        ('I', 34),
        ('P', 35),
        ('L', 37),
        ('J', 38),
        ('K', 40),
        ('N', 45),
        ('M', 46),
    ];
    Some(match key.to_ascii_uppercase().as_str() {
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "5" => 23,
        "6" => 22,
        "7" => 26,
        "8" => 28,
        "9" => 25,
        "0" => 29,
        "=" => 24,
        "-" => 27,
        "]" => 30,
        "[" => 33,
        "'" => 39,
        ";" => 41,
        "\\" => 42,
        "," => 43,
        "/" => 44,
        "." => 47,
        "`" => 50,
        "TAB" => 48,
        "ENTER" | "RETURN" => 36,
        "BACKSPACE" => 51,
        "DELETE" => 117,
        "HOME" => 115,
        "END" => 119,
        "PAGEUP" => 116,
        "PAGEDOWN" => 121,
        "SPACE" => 49,
        "ESC" | "ESCAPE" => 53,
        "LEFT" => 123,
        "RIGHT" => 124,
        "DOWN" => 125,
        "UP" => 126,
        "F1" => 122,
        "F2" => 120,
        "F3" => 99,
        "F4" => 118,
        "F5" => 96,
        "F6" => 97,
        "F7" => 98,
        "F8" => 100,
        "F9" => 101,
        "F10" => 109,
        "F11" => 103,
        "F12" => 111,
        k if k.chars().count() == 1 => {
            let c = k.chars().next()?;
            LETTERS.iter().find(|(l, _)| *l == c)?.1
        }
        _ => return None,
    })
}

const MASK: NSEventModifierFlags = NSEventModifierFlags(
    NSEventModifierFlags::Command.0
        | NSEventModifierFlags::Option.0
        | NSEventModifierFlags::Control.0
        | NSEventModifierFlags::Shift.0,
);

/// The characters AppKit gives a key: control characters and function-key
/// code points for named keys, else the key in lower case.
fn key_chars(key: &str) -> String {
    let c = |u: u32| char::from_u32(u).map(String::from).unwrap_or_default();
    match key.to_ascii_uppercase().as_str() {
        "ENTER" | "RETURN" => "\r".into(),
        "TAB" => "\t".into(),
        "SPACE" => " ".into(),
        "ESC" | "ESCAPE" => c(0x1b),
        "BACKSPACE" => c(0x7f),
        "DELETE" => c(0xF728),
        "UP" => c(0xF700),
        "DOWN" => c(0xF701),
        "LEFT" => c(0xF702),
        "RIGHT" => c(0xF703),
        "HOME" => c(0xF729),
        "END" => c(0xF72B),
        "PAGEUP" => c(0xF72C),
        "PAGEDOWN" => c(0xF72D),
        _ => key.to_ascii_lowercase(),
    }
}

/// `Cmd+Shift+T` as (key code, modifier bits).
pub fn parse(shortcut: &str) -> Option<(u16, usize)> {
    let mut parts: Vec<&str> = shortcut.split('+').collect();
    let key = parts.pop()?;
    let mut flags = NSEventModifierFlags(0);
    for m in parts {
        flags |= match m {
            "Cmd" => NSEventModifierFlags::Command,
            "Alt" => NSEventModifierFlags::Option,
            "Ctrl" => NSEventModifierFlags::Control,
            "Shift" => NSEventModifierFlags::Shift,
            _ => return None,
        };
    }
    Some((key_code(key)?, flags.0))
}

/// The shortcuts of the reserved commands, as the user set them.
pub fn set_reserved(shortcuts: &[String]) {
    RESERVED.with_borrow_mut(|r| *r = shortcuts.iter().filter_map(|s| parse(s)).collect());
}

/// Whether `event` is a reserved shortcut.
pub fn is_reserved(event: &NSEvent) -> bool {
    let key = (event.keyCode(), (event.modifierFlags() & MASK).0);
    RESERVED.with_borrow(|r| r.contains(&key))
}

/// Gives `event` to the menu bar; true when a menu item took it.
pub fn to_menu(event: &NSEvent) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    NSApplication::sharedApplication(mtm)
        .mainMenu()
        .is_some_and(|m| m.performKeyEquivalent(event))
}

/// Presses `shortcut` on a tab's page as a person would: a key-down
/// AppKit event offered to the tab view as a key equivalent, the way the
/// window offers it. For end-to-end tests (spec 16 §3).
pub fn synthesize(tab: mado_types::TabId, shortcut: &str) -> Result<(), mado_engine::EngineError> {
    use mado_engine::EngineError;
    use objc2_app_kit::NSEventType;
    let (code, flags) =
        parse(shortcut).ok_or_else(|| EngineError::Failed("not a shortcut".into()))?;
    let key = key_chars(shortcut.rsplit('+').next().unwrap_or_default());
    let view = crate::with(|a| a.view(tab))
        .flatten()
        .ok_or_else(|| EngineError::Failed("no view for that tab".into()))?;
    let window_number = view.window().map_or(0, |w| w.windowNumber());
    let chars = objc2_foundation::NSString::from_str(&key);
    let time = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        objc2_foundation::NSPoint::new(0.0, 0.0),
        NSEventModifierFlags(flags),
        time,
        window_number,
        None,
        &chars,
        &chars,
        false,
        code,
    )
    .ok_or_else(|| EngineError::Failed("AppKit refused the key event".into()))?;
    if !view.performKeyEquivalent(&event) {
        view.keyDown(&event);
    }
    Ok(())
}

/// Types `text` into the focused element of a tab's page: a key-down and
/// key-up per character, which WebKit turns into trusted `keydown`,
/// `input` and `keyup` events (spec 13 §8).
pub fn type_text(tab: mado_types::TabId, text: &str) -> Result<(), mado_engine::EngineError> {
    use mado_engine::EngineError;
    use objc2_app_kit::NSEventType;
    let view = crate::with(|a| a.view(tab))
        .flatten()
        .ok_or_else(|| EngineError::Failed("no view for that tab".into()))?;
    // A hidden tab may be outside any window; its view still takes keys.
    let window_number = view.window().map_or(0, |w| w.windowNumber());
    let mut buf = [0u8; 4];
    for c in text.chars() {
        let (code, chars) = match c {
            '\n' => (36u16, "\r"),
            '\t' => (48, "\t"),
            c => (0, &*c.encode_utf8(&mut buf)),
        };
        let chars = objc2_foundation::NSString::from_str(chars);
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let time = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                objc2_foundation::NSPoint::new(0.0, 0.0),
                NSEventModifierFlags(0),
                time,
                window_number,
                None,
                &chars,
                &chars,
                false,
                code,
            )
            .ok_or_else(|| EngineError::Failed("AppKit refused the key event".into()))?;
            if kind == NSEventType::KeyDown {
                view.keyDown(&event);
            } else {
                view.keyUp(&event);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_to_physical_keys() {
        let cmd = NSEventModifierFlags::Command.0;
        let shift = NSEventModifierFlags::Shift.0;
        let alt = NSEventModifierFlags::Option.0;
        assert_eq!(parse("Cmd+T"), Some((17, cmd)));
        assert_eq!(parse("Cmd+Shift+T"), Some((17, cmd | shift)));
        assert_eq!(parse("Cmd+Alt+Right"), Some((124, cmd | alt)));
        assert_eq!(parse("Cmd+9"), Some((25, cmd)));
        assert_eq!(parse("Cmd+L"), Some((37, cmd)));
        assert_eq!(parse("Cmd+Hyper+T"), None);
        assert_eq!(parse(""), None);
    }
}
