//! `// @tunable name default min max max_step "description"` line parser.

use mm_core::TunableParam;

use crate::controller::ScriptError;

/// Collect every `@tunable` declaration in `code`. Paths are `controller.<name>`.
pub fn parse_tunables(code: &str) -> Result<Vec<TunableParam>, Vec<ScriptError>> {
    let mut out: Vec<TunableParam> = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in code.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("//") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix("@tunable") else {
            continue;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let col = (line.len() - trimmed.len() + 1) as u32;
        match parse_one(rest) {
            Ok(p) => {
                if out.iter().any(|q| q.path == p.path) {
                    errors.push(err(i, col, format!("duplicate @tunable `{}`", p.path)));
                } else {
                    out.push(p);
                }
            }
            Err(m) => errors.push(err(i, col, format!("bad @tunable: {m}"))),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

fn err(line_idx: usize, col: u32, message: String) -> ScriptError {
    ScriptError {
        line: Some(line_idx as u32 + 1),
        col: Some(col),
        message,
    }
}

const USAGE: &str = "expected `name default min max max_step \"description\"`";

fn parse_one(rest: &str) -> Result<TunableParam, String> {
    let mut rest = rest.trim();
    let mut toks: Vec<&str> = Vec::new();
    for _ in 0..5 {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return Err(USAGE.into());
        }
        toks.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    let name = toks[0];
    let valid_name = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid_name {
        return Err(format!("`{name}` is not a valid identifier"));
    }
    let num = |what: &str, s: &str| -> Result<f64, String> {
        s.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{what} `{s}` is not a finite number"))
    };
    let default = num("default", toks[1])?;
    let min = num("min", toks[2])?;
    let max = num("max", toks[3])?;
    let max_step = num("max_step", toks[4])?;
    if !(rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"')) {
        return Err(format!("{USAGE}; description must be in double quotes"));
    }
    let description = rest[1..rest.len() - 1].replace("\\\"", "\"");
    if min > max {
        return Err(format!("min {min} is above max {max}"));
    }
    if default < min || default > max {
        return Err(format!("default {default} is outside [{min}, {max}]"));
    }
    if max_step <= 0.0 {
        return Err(format!("max_step {max_step} must be positive"));
    }
    Ok(TunableParam {
        path: format!("controller.{name}"),
        description,
        min,
        max,
        default,
        max_step,
    })
}
