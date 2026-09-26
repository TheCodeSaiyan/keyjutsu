//! An independent review of a plan by a second agent.
//!
//! A review cannot change the plan. Its findings are shown to the operator and
//! can be passed to the primary agent as guidance for a revision; they are
//! recorded in provenance as the reviewer challenging the steps they name.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "agent/")]
pub enum FindingKind {
    Assumption,
    MissingValidation,
    Unsafe,
    WeakRollback,
    Alternative,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "agent/")]
pub enum Severity {
    Info,
    Warning,
    Serious,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "agent/")]
pub struct Finding {
    #[serde(default)]
    pub step: Option<String>,
    pub kind: FindingKind,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "agent/")]
pub struct Review {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReviewError {
    #[error("the review is not in the expected shape: {0}")]
    Shape(String),
    #[error("the review names step `{0}`, which is not in the plan")]
    UnknownStep(String),
}

/// Parse a reviewer's answer. Findings about steps that do not exist are
/// refused rather than silently attached to nothing.
pub fn parse(value: Value, step_ids: &[&str]) -> Result<Review, ReviewError> {
    let mut review: Review = serde_json::from_value(value).map_err(|e| ReviewError::Shape(e.to_string()))?;
    for f in &review.findings {
        if let Some(s) = &f.step
            && !step_ids.contains(&s.as_str())
        {
            return Err(ReviewError::UnknownStep(s.clone()));
        }
    }
    review.summary = review.summary.chars().take(4000).collect();
    for f in &mut review.findings {
        f.message = f.message.chars().take(2000).collect();
    }
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_well_formed_review_parses() {
        let r = parse(
            json!({"summary": "Mostly fine.", "findings": [
                {"step": "wsl-path", "kind": "weak_rollback", "severity": "warning", "message": "No way back."},
                {"step": null, "kind": "assumption", "severity": "info", "message": "Assumes WSL 2."}
            ]}),
            &["wsl-path"],
        )
        .unwrap();
        assert_eq!(r.findings.len(), 2);
        assert_eq!(r.findings[0].kind, FindingKind::WeakRollback);
    }

    #[test]
    fn findings_about_missing_steps_or_extra_fields_are_refused() {
        assert_eq!(
            parse(
                json!({"findings": [{"step": "nope", "kind": "other", "severity": "info", "message": "m"}]}),
                &["a"]
            ),
            Err(ReviewError::UnknownStep("nope".into()))
        );
        assert!(matches!(parse(json!({"findings": [], "approve": true}), &[]), Err(ReviewError::Shape(_))));
    }
}
