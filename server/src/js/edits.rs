use oxc_span::Span;

#[derive(Default)]
pub struct Edits {
    edits: Vec<(u32, u32, String)>,
}

impl Edits {
    pub fn insert(&mut self, offset: u32, text: String) {
        self.edits.push((offset, offset, text));
    }

    pub fn blank(&mut self, span: Span, source: &str) {
        let blanked = source[span.start as usize..span.end as usize]
            .chars()
            .map(|character| if character == '\n' { '\n' } else { ' ' })
            .collect();
        self.edits.push((span.start, span.end, blanked));
    }

    pub fn apply(mut self, source: &str) -> String {
        self.edits.sort_by_key(|(start, _, _)| *start);
        let mut result = String::with_capacity(source.len() * 2);
        let mut position = 0;
        for (start, end, text) in self.edits {
            result.push_str(&source[position..start as usize]);
            result.push_str(&text);
            position = end as usize;
        }
        result.push_str(&source[position..]);
        result
    }
}
