use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Traced(Trace),
    NoFunction,
    NoCall { template: Template },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Template {
    pub after_line: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Trace {
    pub function: Function,
    pub loops: Vec<LoopInfo>,
    pub calls: Vec<Call>,
    pub stopped: Option<Stop>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Function {
    pub name: String,
    pub line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LoopInfo {
    pub line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    Steps,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Call {
    pub depth: u32,
    pub scope: Scope,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Scope {
    pub label: Option<String>,
    pub events: Vec<Event>,
    pub loops: Vec<LoopRun>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LoopRun {
    pub line: u32,
    pub iterations: Vec<Scope>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Event {
    pub line: u32,
    pub values: Vec<(String, String)>,
    #[serde(rename = "return")]
    pub returned: Option<String>,
    pub condition: Option<bool>,
    pub exception: Option<String>,
}
