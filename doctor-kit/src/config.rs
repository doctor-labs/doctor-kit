//! `doctor.toml`: score weights and check enable/disable by id.
use doctor_core::Severity;
use serde::Deserialize;

/// Points deducted per finding, by severity (default additive model).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Weights {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub info: u32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights {
            critical: 25,
            high: 10,
            medium: 4,
            low: 1,
            info: 0,
        }
    }
}

impl Weights {
    pub fn of(&self, s: Severity) -> u32 {
        match s {
            Severity::Critical => self.critical,
            Severity::High => self.high,
            Severity::Medium => self.medium,
            Severity::Low => self.low,
            Severity::Info => self.info,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Checks {
    /// When non-empty, only these check ids run.
    pub enable: Vec<String>,
    /// These check ids never run (wins over `enable`).
    pub disable: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub score: Weights,
    pub checks: Checks,
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| format!("doctor.toml: {e}"))
    }

    pub fn check_enabled(&self, id: &str) -> bool {
        !self.checks.disable.iter().any(|d| d == id)
            && (self.checks.enable.is_empty() || self.checks.enable.iter().any(|e| e == id))
    }
}
