use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

pub(super) fn encode_mouse(
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    x: u16,
    y: u16,
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
) -> Vec<u8> {
    use MouseEventKind as M;
    let (mut code, release) = match kind {
        M::Down(button) => (mouse_button(button), false),
        M::Up(button) if mode != vt100::MouseProtocolMode::Press => (mouse_button(button), true),
        M::Drag(button)
            if matches!(
                mode,
                vt100::MouseProtocolMode::ButtonMotion | vt100::MouseProtocolMode::AnyMotion
            ) =>
        {
            (mouse_button(button) + 32, false)
        }
        M::Moved if mode == vt100::MouseProtocolMode::AnyMotion => (35, false),
        M::ScrollUp => (64, false),
        M::ScrollDown => (65, false),
        M::ScrollLeft => (66, false),
        M::ScrollRight => (67, false),
        _ => return Vec::new(),
    };
    if modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }
    match encoding {
        vt100::MouseProtocolEncoding::Sgr => {
            format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
        }
        vt100::MouseProtocolEncoding::Default | vt100::MouseProtocolEncoding::Utf8 => {
            if x > 223 || y > 223 || code > 223 {
                return Vec::new();
            }
            let code = if release { 3 + (code & !32) } else { code };
            vec![
                0x1b,
                b'[',
                b'M',
                32 + code as u8,
                32 + x as u8,
                32 + y as u8,
            ]
        }
    }
}

fn mouse_button(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}
