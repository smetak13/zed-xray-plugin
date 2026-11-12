pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> LineIndex {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index as u32 + 1),
        );
        LineIndex { starts }
    }

    pub fn line(&self, offset: u32) -> u32 {
        (self.starts.partition_point(|start| *start <= offset) - 1) as u32
    }

    pub fn end_line(&self, end: u32) -> u32 {
        self.line(end.saturating_sub(1))
    }
}
