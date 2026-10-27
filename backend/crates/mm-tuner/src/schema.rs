//! Strict JSON schema for [`MuseReport`], wire-format conversion, and the
//! parse-then-repair-once request helper.

use mm_core::{MuseReport, Suggestion, SuggestionTarget};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::provider::{ChatProvider, ChatRequest, Usage};
use crate::TunerError;

pub const REPORT_SCHEMA_NAME: &str = "muse_report";

/// Hand-written to satisfy the Meta strict rules: object root, `additionalProperties:
/// false` and `required` = all keys on every object, no oneOf/allOf/$ref, shallow.
/// `SuggestionTarget` is flattened: `type` selects which of `path`/`new_value`/`diff`
/// matters; the others are `""` / `0`.
pub fn muse_report_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["diagnosis", "root_cause", "evidence", "suggestions"],
        "properties": {
            "diagnosis": {"type": "string"},
            "root_cause": {"type": "string"},
            "evidence": {"type": "array", "items": {"type": "string"}},
            "suggestions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "detail", "target", "expected_effect", "confidence"],
                    "properties": {
                        "title": {"type": "string"},
                        "detail": {"type": "string"},
                        "target": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["type", "path", "new_value", "diff"],
                            "properties": {
                                "type": {"type": "string", "enum": ["param", "code"]},
                                "path": {"type": "string"},
                                "new_value": {"type": "number"},
                                "diff": {"type": "string"}
                            }
                        },
                        "expected_effect": {"type": "string"},
                        "confidence": {"type": "number"}
                    }
                }
            }
        }
    })
}

#[derive(Deserialize)]
struct WireReport {
    diagnosis: String,
    root_cause: String,
    evidence: Vec<String>,
    suggestions: Vec<WireSuggestion>,
}

#[derive(Deserialize)]
struct WireSuggestion {
    title: String,
    detail: String,
    target: WireTarget,
    expected_effect: String,
    confidence: f64,
}

#[derive(Deserialize)]
struct WireTarget {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    new_value: f64,
    #[serde(default)]
    diff: String,
}

/// Strip a markdown fence if the model added one.
fn strip_fence(s: &str) -> &str {
    let t = s.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    t.strip_suffix("```").unwrap_or(t).trim()
}

/// Parse the wire JSON into a [`MuseReport`].
pub fn parse_report(content: &str) -> Result<MuseReport, TunerError> {
    let w: WireReport =
        serde_json::from_str(strip_fence(content)).map_err(|e| TunerError::Parse(e.to_string()))?;
    let mut suggestions = Vec::with_capacity(w.suggestions.len());
    for s in w.suggestions {
        let target = match s.target.kind.as_str() {
            "param" => SuggestionTarget::Param {
                path: s.target.path,
                new_value: s.target.new_value,
            },
            "code" => SuggestionTarget::Code {
                diff: s.target.diff,
            },
            other => {
                return Err(TunerError::Parse(format!(
                    "unknown target type '{other}' (expected param or code)"
                )))
            }
        };
        suggestions.push(Suggestion {
            title: s.title,
            detail: s.detail,
            target,
            expected_effect: s.expected_effect,
            confidence: if s.confidence.is_finite() {
                s.confidence.clamp(0.0, 1.0)
            } else {
                0.0
            },
        });
    }
    Ok(MuseReport {
        diagnosis: w.diagnosis,
        root_cause: w.root_cause,
        evidence: w.evidence,
        suggestions,
    })
}

/// One prompt/response pair, kept for the audit log.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub system: String,
    pub user: String,
    pub response: String,
}

pub struct ReportCall {
    pub report: MuseReport,
    pub usage: Usage,
    pub exchanges: Vec<Exchange>,
}

/// Ask for a report; on a parse failure retry exactly once with the error attached.
pub async fn request_report(
    p: &dyn ChatProvider,
    system: &str,
    user: &str,
    max_tokens: u32,
    temperature: f32,
) -> Result<ReportCall, TunerError> {
    let mut usage = Usage::default();
    let mut exchanges = Vec::new();
    let mut user_text = user.to_string();
    let mut last_err = None;
    for attempt in 0..2 {
        let resp = p
            .complete(ChatRequest {
                system: system.to_string(),
                user: user_text.clone(),
                json_schema: Some((REPORT_SCHEMA_NAME.into(), muse_report_schema())),
                max_tokens,
                temperature,
            })
            .await?;
        usage.add(resp.usage);
        exchanges.push(Exchange {
            system: system.to_string(),
            user: user_text.clone(),
            response: resp.content.clone(),
        });
        match parse_report(&resp.content) {
            Ok(report) => {
                return Ok(ReportCall {
                    report,
                    usage,
                    exchanges,
                })
            }
            Err(e) => {
                if attempt == 0 {
                    let prev: String = resp.content.chars().take(4000).collect();
                    user_text = format!(
                        "{user}\n\nYour previous reply could not be parsed: {e}\nPrevious reply:\n{prev}\n\nReturn corrected JSON only, matching the schema exactly."
                    );
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| TunerError::Parse("no response".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    pub fn valid_json(path: &str, v: f64) -> String {
        json!({
            "diagnosis": "d", "root_cause": "r", "evidence": ["e"],
            "suggestions": [{
                "title": "t", "detail": "x",
                "target": {"type": "param", "path": path, "new_value": v, "diff": ""},
                "expected_effect": "better", "confidence": 0.7
            }]
        })
        .to_string()
    }

    fn walk(v: &Value, depth: usize, max_depth: &mut usize) {
        *max_depth = (*max_depth).max(depth);
        let Value::Object(m) = v else {
            if let Value::Array(a) = v {
                a.iter().for_each(|x| walk(x, depth + 1, max_depth));
            }
            return;
        };
        for banned in ["oneOf", "allOf", "anyOf", "$ref"] {
            assert!(!m.contains_key(banned), "{banned} not allowed");
        }
        if m.get("type") == Some(&json!("object")) {
            assert_eq!(m.get("additionalProperties"), Some(&json!(false)));
            let props: std::collections::BTreeSet<_> = m["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            let req: std::collections::BTreeSet<_> = m["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect();
            assert_eq!(props, req, "required must list all properties");
        }
        m.values().for_each(|x| walk(x, depth + 1, max_depth));
    }

    #[test]
    fn schema_is_strict() {
        let s = muse_report_schema();
        assert_eq!(s["type"], "object");
        let mut d = 0;
        walk(&s, 1, &mut d);
        assert!(d <= 10, "depth {d}");
    }

    #[tokio::test]
    async fn valid_response_parses_with_one_call() {
        let p = MockProvider::scripted([valid_json("controller.wall_dist", 0.12)]);
        let r = request_report(&p, "s", "u", 100, 0.2).await.unwrap();
        assert_eq!(p.call_count(), 1);
        assert_eq!(
            r.report.suggestions[0].target,
            SuggestionTarget::Param {
                path: "controller.wall_dist".into(),
                new_value: 0.12
            }
        );
        assert!(p.requests()[0].json_schema.is_some());
    }

    #[tokio::test]
    async fn invalid_then_valid_retries_exactly_once_with_error() {
        let p = MockProvider::scripted(["not json".to_string(), valid_json("a", 1.0)]);
        let r = request_report(&p, "s", "u", 100, 0.2).await.unwrap();
        assert_eq!(p.call_count(), 2);
        assert_eq!(r.exchanges.len(), 2);
        assert!(p.requests()[1].user.contains("could not be parsed"));
    }

    #[tokio::test]
    async fn two_bad_responses_give_parse_error() {
        let p = MockProvider::scripted(["nope", "still nope", valid_json("a", 1.0).as_str()]);
        let e = request_report(&p, "s", "u", 100, 0.2).await.err().unwrap();
        assert!(matches!(e, TunerError::Parse(_)));
        assert_eq!(p.call_count(), 2);
    }

    #[test]
    fn code_target_converts_and_fences_are_stripped() {
        let j = json!({
            "diagnosis": "d", "root_cause": "r", "evidence": [],
            "suggestions": [{"title": "t", "detail": "x",
              "target": {"type": "code", "path": "", "new_value": 0, "diff": "@@ -1 +1 @@"},
              "expected_effect": "e", "confidence": 3.0}]
        });
        let r = parse_report(&format!("```json\n{j}\n```")).unwrap();
        assert!(matches!(
            r.suggestions[0].target,
            SuggestionTarget::Code { .. }
        ));
        assert_eq!(r.suggestions[0].confidence, 1.0);
    }
}
