//! What needs the operator in a plan, and what they can answer (ADR 0020).
//!
//! Each finding validation recorded against a step, and each reviewer's
//! concern not yet dealt with, becomes an ask: what it is, and the choices
//! that answer it. The choices come from this module's own table, from the
//! kind of finding, never from anything an agent wrote; each is one of the
//! workspace's own operations, so answering can do nothing the operator could
//! not already do. Free text is always possible besides, as guidance for the
//! agent or as the operator's note.

use keyjutsu_plan::model::{EvidenceResult, Plan};
use serde::Serialize;

use crate::workspace::Note;

/// One answer to an ask. The window carries it out with the workspace
/// operation it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "workspace/")]
pub enum Choice {
    /// Download what the plan needs and pin its hash.
    StageDownloads,
    /// Take KeyJutsu's own risk rating for the step.
    UseKeyJutsuRating,
    /// Send `guidance` to the agent: for the step, a retry; for the plan, a
    /// revision. What comes back is unvalidated and unapproved.
    AskAgent { label: String, guidance: String },
    /// The operator changed the machine; check the plan again.
    ValidateAgain,
    /// Open the step in the editor.
    EditStep,
    /// Take the step out of the plan.
    RemoveStep,
    /// A reviewer's concern that needs no change, with the operator's reason.
    Dismiss { note: usize },
}

/// Where an ask came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "workspace/")]
pub enum AskFrom {
    /// KeyJutsu's validation: `check` is what it looked at.
    Validation { check: String },
    /// A reviewer's concern, by name.
    Review { who: String },
}

/// Something in the plan that needs the operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct Ask {
    /// The step it is about; `None` for the whole plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<String>,
    pub from: AskFrom,
    /// What it says, in full.
    pub text: String,
    pub choices: Vec<Choice>,
}

fn ask_agent(label: &str, guidance: String) -> Choice {
    Choice::AskAgent { label: label.into(), guidance }
}

/// The choices for one finding, by the check that found it.
fn choices_for(check: &str, detail: &str) -> Vec<Choice> {
    let about = |what: &str| format!("Validation found: {check}: {detail}\n{what}");
    match check {
        "artifact" => vec![Choice::StageDownloads, Choice::RemoveStep],
        "risk" => vec![
            Choice::UseKeyJutsuRating,
            ask_agent(
                "Ask the agent why",
                about(
                    "KeyJutsu rates this step higher than you did. Explain why it is needed, or make it less risky.",
                ),
            ),
        ],
        "commands" | "tools" | "shell" | "shell version" => vec![
            ask_agent(
                "Ask for a step without it",
                about("Change this step so it works without what is missing here."),
            ),
            Choice::ValidateAgain,
        ],
        "preconditions" | "working directory" => vec![
            ask_agent(
                "Ask the agent to adjust it",
                about("Adjust this step so its precondition holds, or check it another way."),
            ),
            Choice::ValidateAgain,
            Choice::RemoveStep,
        ],
        "privilege" => vec![
            ask_agent(
                "Ask for a step without Administrator",
                about("Change this step so it does not need Administrator."),
            ),
            Choice::RemoveStep,
        ],
        _ => {
            vec![ask_agent("Ask the agent to fix it", about("Fix this step so it passes.")), Choice::EditStep]
        }
    }
}

/// Everything in `plan` that needs the operator: each step's failed findings
/// (and preconditions nobody could decide), in plan order, then reviewers'
/// concerns not yet dismissed.
pub fn asks(plan: &Plan, order: &[&str], notes: &[Note]) -> Vec<Ask> {
    let mut out = Vec::new();
    let states = plan.keyjutsu.as_ref().map(|k| &k.steps);
    for id in order {
        let Some(state) = states.and_then(|s| s.get(*id)) else { continue };
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for e in &state.evidence {
            let detail = e.detail.as_deref().unwrap_or_default();
            let undecided = e.check == "preconditions"
                && e.result == EvidenceResult::NotApplicable
                && detail.starts_with("could not be decided");
            if (e.result != EvidenceResult::Failed && !undecided)
                || seen.contains(&(e.check.as_str(), detail))
            {
                continue;
            }
            seen.push((&e.check, detail));
            out.push(Ask {
                step: Some((*id).to_owned()),
                from: AskFrom::Validation { check: e.check.clone() },
                text: format!("{}: {detail}", e.check),
                choices: choices_for(&e.check, detail),
            });
        }
    }
    for (i, n) in notes.iter().enumerate().filter(|(_, n)| n.review && !n.dismissed) {
        let guidance = format!(
            "{} raised this about the plan: {}\nDeal with it, or explain why it is fine.",
            n.who, n.text
        );
        out.push(Ask {
            step: n.step.clone(),
            from: AskFrom::Review { who: n.who.clone() },
            text: n.text.clone(),
            choices: if n.step.is_some() {
                vec![
                    ask_agent("Ask the agent to address it", guidance),
                    Choice::Dismiss { note: i },
                    Choice::EditStep,
                ]
            } else {
                vec![ask_agent("Ask the agent to address it", guidance), Choice::Dismiss { note: i }]
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(choices: &[Choice]) -> Vec<&'static str> {
        choices
            .iter()
            .map(|c| match c {
                Choice::StageDownloads => "stage",
                Choice::UseKeyJutsuRating => "rating",
                Choice::AskAgent { .. } => "agent",
                Choice::ValidateAgain => "validate",
                Choice::EditStep => "edit",
                Choice::RemoveStep => "remove",
                Choice::Dismiss { .. } => "dismiss",
            })
            .collect()
    }

    /// The table in ADR 0020: each kind of finding, and what answers it.
    #[test]
    fn each_kind_of_finding_has_its_own_answers() {
        for (check, expected) in [
            ("artifact", &["stage", "remove"][..]),
            ("risk", &["rating", "agent"]),
            ("commands", &["agent", "validate"]),
            ("tools", &["agent", "validate"]),
            ("shell", &["agent", "validate"]),
            ("preconditions", &["agent", "validate", "remove"]),
            ("working directory", &["agent", "validate", "remove"]),
            ("privilege", &["agent", "remove"]),
            ("syntax", &["agent", "edit"]),
            ("dry run", &["agent", "edit"]),
        ] {
            assert_eq!(kinds(&choices_for(check, "x")), expected, "{check}");
        }
    }

    /// What goes to the agent says what was found, so a retry starts from it.
    #[test]
    fn asking_the_agent_carries_the_finding() {
        let choices = choices_for("commands", "`magick` was not found");
        let Some(Choice::AskAgent { guidance, .. }) = choices.first() else { panic!("{choices:?}") };
        assert!(guidance.contains("commands: `magick` was not found"), "{guidance}");
    }

    fn note(step: Option<&str>, review: bool, dismissed: bool) -> Note {
        Note {
            who: "codex".into(),
            text: "Nothing checks it worked.".into(),
            step: step.map(Into::into),
            review,
            dismissed,
            at: "2026-09-27T00:00:00Z".into(),
        }
    }

    fn plan() -> Plan {
        serde_json::from_value(serde_json::json!({
            "schema_version": "1.0", "plan_id": "p", "task_id": "t",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "codex", "version": "1"},
            "steps": [{"id": "a", "title": "A", "objective": "A.", "kind": "manual"}]
        }))
        .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Only concerns still open are asked about; the operator's own notes
    /// never are; a concern about the whole plan has no step to edit.
    #[test]
    fn only_open_concerns_are_asked_about() {
        let notes = [
            note(Some("a"), false, false),
            note(Some("a"), true, false),
            note(Some("a"), true, true),
            note(None, true, false),
        ];
        let asks = asks(&plan(), &["a"], &notes);
        assert_eq!(asks.len(), 2, "{asks:?}");
        assert_eq!(asks[0].step.as_deref(), Some("a"));
        assert_eq!(kinds(&asks[0].choices), ["agent", "dismiss", "edit"]);
        assert_eq!(asks[0].choices[1], Choice::Dismiss { note: 1 }, "it names the note it dismisses");
        assert_eq!(asks[1].step, None);
        assert_eq!(kinds(&asks[1].choices), ["agent", "dismiss"]);
    }
}
