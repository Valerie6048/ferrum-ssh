//! Snippet hotkeys: "Cmd+Alt+1" <-> egui modifiers + key.

use egui::{Key, Modifiers};

/// Parse text such as `Cmd+Shift+K`. `Cmd` is ⌘ on macOS and Ctrl elsewhere;
/// `Ctrl` is the physical Ctrl key. At least one modifier is required so a
/// hotkey can never swallow ordinary typing.
pub fn parse(text: &str) -> Option<(Modifiers, Key)> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "mod" | "super" | "meta" => mods |= Modifiers::COMMAND,
            "ctrl" | "control" => mods |= Modifiers::CTRL,
            "alt" | "opt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(Key::from_name(part)?);
            }
        }
    }
    let key = key?;
    (mods != Modifiers::NONE).then_some((mods, key))
}

pub fn format(mods: Modifiers, key: Key) -> String {
    let mut parts: Vec<&str> = vec![];
    if mods.command {
        parts.push("Cmd");
    } else if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    parts.push(key.name());
    parts.join("+")
}

/// Shortcuts the app itself uses; snippets may not take them over.
pub fn is_reserved(mods: Modifiers, key: Key) -> bool {
    const PLAIN: [Key; 16] = [
        Key::K,
        Key::T,
        Key::W,
        Key::D,
        Key::Comma,
        Key::Equals,
        Key::Plus,
        Key::Minus,
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
    ];
    let cmd_only = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    (mods == cmd_only && (PLAIN.contains(&key) || matches!(key, Key::Num8 | Key::Num9 | Key::Tab)))
        || (mods == cmd_shift && matches!(key, Key::D | Key::P))
        || (mods == Modifiers::CTRL && key == Key::Tab)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_key() {
        let (m, k) = parse("Cmd+Alt+1").unwrap();
        assert!(m.command && m.alt && !m.shift);
        assert_eq!(k, Key::Num1);
    }

    #[test]
    fn rejects_bare_keys_and_garbage() {
        assert!(parse("A").is_none(), "no modifier");
        assert!(parse("Cmd+").is_none(), "no key");
        assert!(parse("Cmd+A+B").is_none(), "two keys");
        assert!(parse("Cmd+Nonsense").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn app_shortcuts_are_reserved() {
        let (m, k) = parse("Cmd+K").unwrap();
        assert!(is_reserved(m, k));
        let (m, k) = parse("Cmd+Alt+K").unwrap();
        assert!(!is_reserved(m, k));
    }

    #[test]
    fn round_trips_through_format() {
        for text in ["Cmd+Shift+K", "Ctrl+Alt+F5", "Cmd+Alt+1"] {
            let (m, k) = parse(text).unwrap();
            assert_eq!(parse(&format(m, k)), Some((m, k)), "{text}");
        }
    }
}
