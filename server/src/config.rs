use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::tracer::Limits;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub python: Option<PathBuf>,
    pub timeout_ms: u64,
    pub max_steps: u64,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            python: None,
            timeout_ms: 5000,
            max_steps: 100_000,
        }
    }
}

impl Config {
    pub fn from_options(options: Option<Value>) -> Config {
        options
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    pub fn limits(&self) -> Limits {
        Limits {
            timeout: Duration::from_millis(self.timeout_ms),
            max_steps: self.max_steps,
        }
    }
}
