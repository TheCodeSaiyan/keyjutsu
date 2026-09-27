//! What needs the operator in a plan, and what they can answer (ADR 0020).
//!
//! Each finding validation recorded against a step, each question the agent
//! asked, and each reviewer's concern not yet dealt with, becomes an ask:
//! what it is, and the choices that answer it. For findings and concerns the
//! choices come from this module's own table, from the kind of finding, never
//! from anything an agent wrote. A question's options are the agent's words,
//! but what choosing one does is fixed here: the answer goes back to the
//! agent as guidance, and what it returns is a change like any other,
//! unvalidated and unapproved. So answering can do nothing the operator could
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
    /// Answer the agent's question `question` with one of its options.
    Answer { question: String, answer: String },
    /// Answer the agent's question in the operator's own words.
    AnswerInOwnWords { question: String },
    /// Leave the plan as it is: the question is closed, and the agent is not
    /// asked anything.
    CarryOn { question: String },
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
    /// A question from the plan's author, by name.
    Agent { who: String },
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
/// (and preconditions nobody could decide), in plan order, then the agent's
/// questions, then reviewers' concerns not yet dismissed.
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
    let author = serde_json::to_value(plan.agent.name)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "The agent".into());
    for q in &plan.questions {
        let mut choices: Vec<Choice> =
            q.options.iter().map(|o| Choice::Answer { question: q.id.clone(), answer: o.clone() }).collect();
        if q.free_text || q.options.is_empty() {
            choices.push(Choice::AnswerInOwnWords { question: q.id.clone() });
        }
        choices.push(Choice::CarryOn { question: q.id.clone() });
        out.push(Ask {
            step: q.step.clone(),
            from: AskFrom::Agent { who: author.clone() },
            text: q.text.clone(),
            choices,
        });
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
                Choice::Answer { .. } => "answer",
                Choice::AnswerInOwnWords { .. } => "own words",
                Choice::CarryOn { .. } => "carry on",
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

    fn asking(questions: serde_json::Value) -> Plan {
        let mut p = serde_json::to_value(plan()).unwrap_or_else(|e| panic!("{e}"));
        p["questions"] = questions;
        serde_json::from_value(p).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Each option is an answer; the operator's own words are offered where
    /// the agent allows them or gave no options; carrying on is always there.
    #[test]
    fn an_agents_question_is_answered_by_its_options() {
        let p = asking(serde_json::json!([
            {"id": "where", "step": "a", "text": "Which desktop?", "options": ["OneDrive", "Local"], "free_text": true},
            {"id": "open", "text": "Open it afterwards?", "options": ["Yes", "No"]},
            {"id": "name", "text": "What should it be called?"}
        ]));
        let asks = asks(&p, &["a"], &[]);
        assert_eq!(asks.len(), 3, "{asks:?}");
        assert_eq!(asks[0].from, AskFrom::Agent { who: "codex".into() });
        assert_eq!(asks[0].step.as_deref(), Some("a"));
        assert_eq!(kinds(&asks[0].choices), ["answer", "answer", "own words", "carry on"]);
        assert_eq!(asks[0].choices[1], Choice::Answer { question: "where".into(), answer: "Local".into() });
        assert_eq!(asks[1].step, None);
        assert_eq!(kinds(&asks[1].choices), ["answer", "answer", "carry on"], "no free text unless allowed");
        assert_eq!(kinds(&asks[2].choices), ["own words", "carry on"], "no options: own words");
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
