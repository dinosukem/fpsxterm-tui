use ratatui::layout::Rect;

use crate::app::Screen;

pub fn terminal_content_rect(area: Rect) -> Rect {
    if area.height < 10 || area.width < 3 {
        return Rect::default();
    }
    Rect::new(
        1,
        6,
        area.width.saturating_sub(2),
        area.height.saturating_sub(9),
    )
}

pub fn workspace_tab_at(column: u16) -> Option<Screen> {
    let mut x = 1u16;
    for (index, screen) in Screen::ALL.into_iter().enumerate() {
        // Tabs adds its own padding around each already-padded title line.
        let width = screen.title().len() as u16 + 4;
        if column >= x && column < x + width {
            return Some(screen);
        }
        let divider_width = if index + 1 < Screen::ALL.len() { 1 } else { 0 };
        x = x.saturating_add(width + divider_width);
    }
    None
}

pub fn file_context_menu_item_at(column: u16, row: u16, area: Rect) -> Option<usize> {
    let rect = super::centered(48, 12, area);
    if column <= rect.x || column >= rect.right().saturating_sub(1) || row <= rect.y {
        return None;
    }
    let index = usize::from(row - rect.y - 1);
    (index < 7 && row < rect.bottom().saturating_sub(1)).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::{file_context_menu_item_at, workspace_tab_at};
    use crate::app::Screen;
    use ratatui::layout::Rect;

    #[test]
    fn workspace_tabs_match_rendered_padding_and_dividers() {
        assert_eq!(workspace_tab_at(1), Some(Screen::Hosts));
        assert_eq!(workspace_tab_at(11), Some(Screen::Terminal));
        assert_eq!(workspace_tab_at(24), Some(Screen::Files));
        assert_eq!(workspace_tab_at(34), Some(Screen::Transfers));
        assert_eq!(workspace_tab_at(48), Some(Screen::History));
        assert_eq!(workspace_tab_at(59), None);
    }

    #[test]
    fn context_menu_hit_test_matches_rendered_item_rows() {
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(file_context_menu_item_at(27, 10, area), Some(0));
        assert_eq!(file_context_menu_item_at(72, 16, area), Some(6));
        assert_eq!(file_context_menu_item_at(26, 10, area), None);
        assert_eq!(file_context_menu_item_at(27, 17, area), None);
    }
}
