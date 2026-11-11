//! Encode crossterm key events into the bytes a terminal would send.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn ctrl_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn modifier_param(m: KeyModifiers) -> u8 {
    1 + m.contains(KeyModifiers::SHIFT) as u8
        + 2 * m.contains(KeyModifiers::ALT) as u8
        + 4 * m.contains(KeyModifiers::CONTROL) as u8
}

pub fn encode(k: &KeyEvent, app_cursor: bool) -> Vec<u8> {
    let m = k.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);
    let has_mods = ctrl || alt || shift;

    let csi_letter = |letter: char| -> Vec<u8> {
        if has_mods {
            format!("\x1b[1;{}{letter}", modifier_param(m)).into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if has_mods {
            format!("\x1b[{n};{}~", modifier_param(m)).into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    let mut out = match k.code {
        KeyCode::Char(c) if ctrl => ctrl_byte(c).map(|b| vec![b]).unwrap_or_default(),
        KeyCode::Char(c) => c.to_string().into_bytes(),
        // Shift/Alt+Enter: Claude Code treats ESC CR as "newline in prompt".
        KeyCode::Enter if shift || alt => return b"\x1b\r".to_vec(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab if shift => return b"\x1b[Z".to_vec(),
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return csi_letter('A'),
        KeyCode::Down => return csi_letter('B'),
        KeyCode::Right => return csi_letter('C'),
        KeyCode::Left => return csi_letter('D'),
        KeyCode::Home => return csi_letter('H'),
        KeyCode::End => return csi_letter('F'),
        KeyCode::Insert => return tilde(2),
        KeyCode::Delete => return tilde(3),
        KeyCode::PageUp => return tilde(5),
        KeyCode::PageDown => return tilde(6),
        KeyCode::F(n) => {
            return match n {
                1..=4 => {
                    let l = b"PQRS"[n as usize - 1] as char;
                    if has_mods {
                        format!("\x1b[1;{}{l}", modifier_param(m)).into_bytes()
                    } else {
                        format!("\x1bO{l}").into_bytes()
                    }
                }
                5 => tilde(15),
                6..=10 => tilde(n + 11),
                11 | 12 => tilde(n + 12),
                _ => vec![],
            }
        }
        _ => vec![],
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn basics() {
        assert_eq!(encode(&k(KeyCode::Char('a'), KeyModifiers::CONTROL), false), vec![1]);
        assert_eq!(encode(&k(KeyCode::Char('x'), KeyModifiers::ALT), false), b"\x1bx");
        assert_eq!(encode(&k(KeyCode::Up, KeyModifiers::NONE), true), b"\x1bOA");
        assert_eq!(encode(&k(KeyCode::Up, KeyModifiers::NONE), false), b"\x1b[A");
        assert_eq!(encode(&k(KeyCode::Left, KeyModifiers::CONTROL), false), b"\x1b[1;5D");
        assert_eq!(encode(&k(KeyCode::Enter, KeyModifiers::SHIFT), false), b"\x1b\r");
        assert_eq!(encode(&k(KeyCode::Char('é'), KeyModifiers::NONE), false), "é".as_bytes());
    }
}
