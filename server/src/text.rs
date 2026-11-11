use tower_lsp_server::ls_types::{Position, Range};

pub fn apply_change(text: &mut String, range: Option<Range>, new_text: &str) {
    match range {
        None => *text = new_text.to_string(),
        Some(range) => {
            let start = byte_offset(text, range.start);
            let end = byte_offset(text, range.end).max(start);
            text.replace_range(start..end, new_text);
        }
    }
}

fn byte_offset(text: &str, position: Position) -> usize {
    let mut line_start = 0;
    for _ in 0..position.line {
        match text[line_start..].find('\n') {
            Some(newline) => line_start += newline + 1,
            None => return text.len(),
        }
    }
    let mut units = 0;
    for (index, character) in text[line_start..].char_indices() {
        if character == '\n' || units >= position.character {
            return line_start + index;
        }
        units += character.len_utf16() as u32;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Option<Range> {
        Some(Range::new(Position::new(sl, sc), Position::new(el, ec)))
    }

    #[test]
    fn full_replacement_without_range() {
        let mut text = "old".to_string();

        apply_change(&mut text, None, "new");

        assert_eq!(text, "new");
    }

    #[test]
    fn replaces_range_within_a_line() {
        let mut text = "let a = 1;\nlet b = 2;\n".to_string();

        apply_change(&mut text, range(1, 8, 1, 9), "42");

        assert_eq!(text, "let a = 1;\nlet b = 42;\n");
    }

    #[test]
    fn inserts_new_line() {
        let mut text = "a\nb\n".to_string();

        apply_change(&mut text, range(1, 0, 1, 0), "x\n");

        assert_eq!(text, "a\nx\nb\n");
    }

    #[test]
    fn deletes_across_lines() {
        let mut text = "a\nb\nc\n".to_string();

        apply_change(&mut text, range(0, 1, 2, 0), "");

        assert_eq!(text, "ac\n");
    }

    #[test]
    fn counts_columns_in_utf16_units() {
        let mut text = "😀x\n".to_string();

        apply_change(&mut text, range(0, 2, 0, 3), "y");

        assert_eq!(text, "😀y\n");
    }

    #[test]
    fn clamps_positions_past_the_end() {
        let mut text = "ab".to_string();

        apply_change(&mut text, range(5, 0, 5, 0), "c");

        assert_eq!(text, "abc");
    }
}
