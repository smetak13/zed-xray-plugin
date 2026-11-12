use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Analysis {
    pub loops: BTreeMap<u32, LoopInfo>,
    pub assigned: BTreeMap<u32, BTreeSet<String>>,
    pub conditions: BTreeMap<u32, (u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoopInfo {
    pub line: u32,
    pub end_line: u32,
    pub body_start: u32,
    pub body_end: u32,
    pub targets: Option<Vec<String>>,
}
