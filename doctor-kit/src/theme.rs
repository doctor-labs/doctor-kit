//! Semantic color tokens. Faces ask for a token, never a raw color.
use doctor_core::Severity;
use owo_colors::{DynColors, OwoColorize, Style};
use std::collections::BTreeMap;

pub const TOKENS: [&str; 9] = [
    "ok", "info", "low", "medium", "high", "critical", "heading", "dim", "accent",
];

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    /// Master switch: false renders every token as plain text.
    pub enabled: bool,
    styles: BTreeMap<&'static str, Style>,
}

/// Parse "bold bright_red", "dim", "#ff8800 underline" into a style.
fn parse_style(spec: &str) -> Result<Style, String> {
    let mut s = Style::new();
    for w in spec.split_whitespace() {
        s = match w {
            "bold" => s.bold(),
            "dim" => s.dimmed(),
            "underline" => s.underline(),
            "italic" => s.italic(),
            "none" => s,
            _ => s.color(parse_color(w).ok_or_else(|| format!("unknown style word `{w}`"))?),
        };
    }
    Ok(s)
}

fn parse_color(w: &str) -> Option<DynColors> {
    use owo_colors::AnsiColors::*;
    if let Some(hex) = w.strip_prefix('#') {
        let n = u32::from_str_radix(hex, 16)
            .ok()
            .filter(|_| hex.len() == 6)?;
        return Some(DynColors::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
    }
    Some(DynColors::Ansi(match w {
        "black" => Black,
        "red" => Red,
        "green" => Green,
        "yellow" => Yellow,
        "blue" => Blue,
        "magenta" => Magenta,
        "cyan" => Cyan,
        "white" => White,
        "bright_black" | "gray" => BrightBlack,
        "bright_red" => BrightRed,
        "bright_green" => BrightGreen,
        "bright_yellow" => BrightYellow,
        "bright_blue" => BrightBlue,
        "bright_magenta" => BrightMagenta,
        "bright_cyan" => BrightCyan,
        "bright_white" => BrightWhite,
        _ => return None,
    }))
}

impl Theme {
    fn build(name: &str, specs: [&str; 9]) -> Theme {
        let styles = TOKENS
            .iter()
            .zip(specs)
            .map(|(t, s)| (*t, parse_style(s).expect("built-in theme spec")))
            .collect();
        Theme {
            name: name.into(),
            enabled: true,
            styles,
        }
    }

    /// No hue at all: weight and dimming only.
    pub fn mono() -> Theme {
        Self::build(
            "mono",
            [
                "none",
                "none",
                "none",
                "none",
                "bold",
                "bold underline",
                "bold",
                "dim",
                "bold",
            ],
        )
    }
    /// Green / amber / red.
    pub fn clinical() -> Theme {
        Self::build(
            "clinical",
            [
                "green",
                "cyan",
                "yellow",
                "yellow bold",
                "red",
                "bold red",
                "bold",
                "dim",
                "cyan",
            ],
        )
    }
    /// Bright, saturated, bold: for dark terminals and low-vision use.
    pub fn contrast() -> Theme {
        Self::build(
            "contrast",
            [
                "bold bright_green",
                "bold bright_cyan",
                "bold bright_white",
                "bold bright_yellow",
                "bold bright_magenta",
                "bold bright_red underline",
                "bold bright_white underline",
                "bright_black",
                "bold bright_blue",
            ],
        )
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        match name {
            "mono" => Some(Self::mono()),
            "clinical" => Some(Self::clinical()),
            "contrast" => Some(Self::contrast()),
            _ => None,
        }
    }

    /// Overlay a `theme.toml` (`base = "clinical"` plus `token = "style"` lines) onto `self`.
    /// A `base` key replaces the starting theme first.
    pub fn with_toml(mut self, text: &str) -> Result<Theme, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("theme.toml: {e}"))?;
        if let Some(b) = table.get("base").and_then(|v| v.as_str()) {
            let on = self.enabled;
            self = Self::by_name(b).ok_or_else(|| format!("theme.toml: unknown base `{b}`"))?;
            self.enabled = on;
        }
        for (k, v) in &table {
            if k == "base" {
                continue;
            }
            let tok = TOKENS
                .iter()
                .find(|t| *t == k)
                .ok_or_else(|| format!("theme.toml: unknown token `{k}`"))?;
            let spec = v
                .as_str()
                .ok_or_else(|| format!("theme.toml: `{k}` must be a string"))?;
            self.styles.insert(tok, parse_style(spec)?);
        }
        Ok(self)
    }

    /// Paint `text` with a token. Unknown tokens and disabled themes return the text as is.
    pub fn paint(&self, token: &str, text: &str) -> String {
        match self.styles.get(token) {
            Some(s) if self.enabled => text.style(*s).to_string(),
            _ => text.to_string(),
        }
    }

    pub fn severity_token(sev: Severity) -> &'static str {
        sev.as_str()
    }

    /// Token for a 0-100 score.
    pub fn score_token(value: u8) -> &'static str {
        match value {
            90.. => "ok",
            60..=89 => "medium",
            _ => "critical",
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::clinical()
    }
}
