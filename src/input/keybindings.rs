use crossterm::event::{KeyCode, KeyModifiers};

pub fn is_previous_screen(code: KeyCode, modifiers: KeyModifiers) -> bool {
    (code == KeyCode::BackTab && modifiers.contains(KeyModifiers::CONTROL))
        || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT))
}

#[cfg(test)]
mod tests {
    use super::is_previous_screen;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn ctrl_shift_tab_is_recognized_in_both_terminal_encodings() {
        assert!(is_previous_screen(
            KeyCode::BackTab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
        assert!(is_previous_screen(
            KeyCode::Tab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
        assert!(!is_previous_screen(KeyCode::Tab, KeyModifiers::CONTROL));
    }
}
