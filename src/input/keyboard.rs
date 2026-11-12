use crossterm::event::{KeyCode, KeyModifiers};

pub(super) fn key_to_bytes(key: crossterm::event::KeyEvent, application_cursor: bool) -> Vec<u8> {
    use KeyCode::*;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let Char(c @ 'a'..='z') = key.code {
            return vec![c as u8 - b'a' + 1];
        }
        if let Char(c @ 'A'..='Z') = key.code {
            return vec![c as u8 - b'A' + 1];
        }
        return match key.code {
            Char(' ') => vec![0],
            Char('[') => vec![0x1b],
            Char('\\') => vec![0x1c],
            Char(']') => vec![0x1d],
            _ => vec![],
        };
    }
    let mut bytes = match key.code {
        Char(c) => {
            let mut buf = [0; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        Enter => vec![b'\r'],
        Tab => vec![b'\t'],
        Backspace => vec![0x7f],
        Esc => vec![0x1b],
        Up => {
            if application_cursor {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        Down => {
            if application_cursor {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        Right => {
            if application_cursor {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        Left => {
            if application_cursor {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        Home => b"\x1b[H".to_vec(),
        End => b"\x1b[F".to_vec(),
        PageUp => b"\x1b[5~".to_vec(),
        PageDown => b"\x1b[6~".to_vec(),
        Delete => b"\x1b[3~".to_vec(),
        Insert => b"\x1b[2~".to_vec(),
        F(n @ 1..=4) => format!("\x1b[{}~", 10 + n).into_bytes(),
        F(5) => b"\x1b[15~".to_vec(),
        F(6) => b"\x1b[17~".to_vec(),
        F(7) => b"\x1b[18~".to_vec(),
        F(8) => b"\x1b[19~".to_vec(),
        F(9) => b"\x1b[20~".to_vec(),
        F(10) => b"\x1b[21~".to_vec(),
        F(11) => b"\x1b[23~".to_vec(),
        F(12) => b"\x1b[24~".to_vec(),
        _ => vec![],
    };
    if key.modifiers.contains(KeyModifiers::ALT) && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}
