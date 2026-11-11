use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use crate::trace::{Event, Scope, Trace};

const LINE_WIDTH: usize = 80;
const PART_SEPARATOR: &str = "  ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Noop,
    Clear,
    PrevCall,
    NextCall,
    PrevIteration(u32),
    NextIteration(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lens {
    pub line: u32,
    pub title: String,
    pub action: Action,
}

impl Lens {
    fn new(line: u32, title: impl Into<String>, action: Action) -> Lens {
        Lens {
            line,
            title: title.into(),
            action,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Placement {
    Values,
    Navigation,
    Controls,
}

pub struct View {
    trace: Trace,
    call: usize,
    cursors: HashMap<u32, usize>,
    edits: Vec<LineEdit>,
}

struct LineEdit {
    touched: Range<u32>,
    first_shifted: u32,
    delta: i64,
}

struct Selection<'a> {
    events: BTreeMap<u32, &'a Event>,
    loops: Vec<LoopPosition<'a>>,
}

struct LoopPosition<'a> {
    line: u32,
    index: usize,
    count: usize,
    label: Option<&'a str>,
}

fn event_parts(event: &Event) -> Vec<String> {
    let mut parts: Vec<String> = event
        .values
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect();
    if let Some(outcome) = event.condition {
        parts.push(format!("→ {}", if outcome { "True" } else { "False" }));
    }
    if let Some(value) = &event.returned {
        parts.push(format!("↩ {value}"));
    }
    if let Some(message) = &event.exception {
        parts.push(format!("💥 {message}"));
    }
    parts
}

fn wrap(parts: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for part in parts {
        match lines.last_mut() {
            Some(last)
                if last.chars().count() + PART_SEPARATOR.len() + part.chars().count()
                    <= LINE_WIDTH =>
            {
                last.push_str(PART_SEPARATOR);
                last.push_str(part);
            }
            _ => lines.push(part.clone()),
        }
    }
    lines.join("\n")
}

impl View {
    pub fn new(trace: Trace) -> View {
        View {
            trace,
            call: 0,
            cursors: HashMap::new(),
            edits: Vec::new(),
        }
    }

    pub fn act(&mut self, action: Action) {
        match action {
            Action::PrevIteration(line) => self.move_loop(line, -1),
            Action::NextIteration(line) => self.move_loop(line, 1),
            Action::PrevCall => self.move_call(-1),
            Action::NextCall => self.move_call(1),
            Action::Noop | Action::Clear => {}
        }
    }

    pub fn replace(&mut self, trace: Trace) {
        self.call = self.call.min(trace.calls.len().saturating_sub(1));
        self.trace = trace;
        self.edits.clear();
    }

    pub fn function_line(&self) -> Option<u32> {
        self.current_line(self.trace.function.line)
    }

    pub fn function_name(&self) -> &str {
        &self.trace.function.name
    }

    pub fn apply_edit(&mut self, start: (u32, u32), end: (u32, u32), text: &str) {
        let added = text.matches('\n').count() as i64;
        let delta = added - i64::from(end.0 - start.0);
        let inserts_whole_lines = start == end && start.1 == 0 && text.ends_with('\n');
        self.edits.push(if inserts_whole_lines {
            LineEdit {
                touched: start.0..start.0,
                first_shifted: start.0,
                delta,
            }
        } else {
            LineEdit {
                touched: start.0..end.0 + 1,
                first_shifted: end.0 + 1,
                delta,
            }
        });
    }

    fn current_line(&self, original: u32) -> Option<u32> {
        self.edits.iter().try_fold(original, |line, edit| {
            if edit.touched.contains(&line) {
                None
            } else if line >= edit.first_shifted {
                u32::try_from(i64::from(line) + edit.delta).ok()
            } else {
                Some(line)
            }
        })
    }

    fn move_call(&mut self, step: isize) {
        let last = self.trace.calls.len().saturating_sub(1);
        let target = self.call.saturating_add_signed(step).min(last);
        if target != self.call {
            self.call = target;
            self.cursors.clear();
        }
    }

    fn move_loop(&mut self, line: u32, step: isize) {
        let Some(position) = self.select().loops.into_iter().find(|run| run.line == line) else {
            return;
        };
        let target = position
            .index
            .saturating_add_signed(step)
            .min(position.count - 1);
        if target == position.index {
            return;
        }
        self.cursors.insert(line, target);
        let nested: Vec<u32> = self
            .trace
            .loops
            .iter()
            .filter(|info| info.line > line)
            .filter(|outer| {
                self.trace
                    .loops
                    .iter()
                    .any(|info| info.line == line && outer.line <= info.end_line)
            })
            .map(|info| info.line)
            .collect();
        for inner in nested {
            self.cursors.remove(&inner);
        }
    }

    fn select(&self) -> Selection<'_> {
        let mut selection = Selection {
            events: BTreeMap::new(),
            loops: Vec::new(),
        };
        if let Some(call) = self.trace.calls.get(self.call) {
            self.walk(&call.scope, &mut selection);
        }
        selection
    }

    fn walk<'a>(&self, scope: &'a Scope, selection: &mut Selection<'a>) {
        for event in &scope.events {
            selection.events.insert(event.line, event);
        }
        for run in &scope.loops {
            let count = run.iterations.len();
            let index = self
                .cursors
                .get(&run.line)
                .copied()
                .unwrap_or(0)
                .min(count - 1);
            let iteration = &run.iterations[index];
            selection.loops.push(LoopPosition {
                line: run.line,
                index,
                count,
                label: iteration.label.as_deref(),
            });
            self.walk(iteration, selection);
        }
    }

    pub fn lenses(&self, document_lines: u32) -> Vec<Lens> {
        let def_line = self.trace.function.line;
        let mut placed = Vec::new();
        if let Some(call) = self
            .trace
            .calls
            .get(self.call)
            .filter(|_| self.trace.calls.len() > 1)
        {
            let title = format!(
                "call {}/{} · depth {}",
                self.call + 1,
                self.trace.calls.len(),
                call.depth
            );
            for lens in [
                Lens::new(def_line, "◀", Action::PrevCall),
                Lens::new(def_line, title, Action::Noop),
                Lens::new(def_line, "▶", Action::NextCall),
            ] {
                placed.push((Placement::Navigation, lens));
            }
        }
        placed.push((Placement::Controls, Lens::new(def_line, "✕", Action::Clear)));
        let selection = self.select();
        for (line, event) in selection.events {
            let parts = event_parts(event);
            if parts.is_empty() {
                continue;
            }
            placed.push((
                Placement::Values,
                Lens::new(line, wrap(&parts), Action::Noop),
            ));
        }
        for position in selection.loops {
            let counter = format!("{}/{}", position.index + 1, position.count);
            let title = match position.label {
                Some(label) if !label.is_empty() => format!("{label} · {counter}"),
                _ => counter,
            };
            for lens in [
                Lens::new(position.line, "◀", Action::PrevIteration(position.line)),
                Lens::new(position.line, title, Action::Noop),
                Lens::new(position.line, "▶", Action::NextIteration(position.line)),
            ] {
                placed.push((Placement::Navigation, lens));
            }
        }
        let mut shown: Vec<(Placement, Lens)> = placed
            .into_iter()
            .filter_map(|(placement, lens)| {
                let line = self.current_line(lens.line)?;
                let line = match placement {
                    Placement::Values if line + 1 < document_lines => line + 1,
                    _ => line,
                };
                Some((placement, Lens { line, ..lens }))
            })
            .collect();
        shown.sort_by_key(|(placement, lens)| (lens.line, *placement));
        shown.into_iter().map(|(_, lens)| lens).collect()
    }
}
