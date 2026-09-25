//! What KeyJutsu asks agents. Every prompt restates the ground rules, because
//! each request is a fresh, stateless run of the agent's CLI.

use crate::context::PreparedContext;

const PLAN_SCHEMA: &str = include_str!("../../../schemas/plan/v1/plan.schema.json");

const RULES: &str = "\
You are helping KeyJutsu, a Windows tool that turns a task into a structured,
validated plan which a human reviews and approves before anything runs.

Rules you must follow:
1. Investigate only. You may read files and run read-only commands to
   understand the machine. Do not change anything: no writes, no installs, no
   service changes, no network calls that alter anything. Everything that
   changes the machine goes in the plan instead; KeyJutsu runs it later, only
   after the operator approves it.
2. Answer with a single JSON document in a ```json fenced block, and nothing
   after it except an optional one-paragraph summary.
3. Each command is one line of text, bound to a shell: pwsh (preferred),
   windows_powershell or cmd. No newlines or control characters inside a
   command.
4. Conditions use only this closed vocabulary, one key per object: all, any,
   not, step_outcome, exit_code, fact, tool_version, path_exists,
   service_state. There is no expression or script form.
5. Never include a \"keyjutsu\" property: readiness, proof, hashes and approval
   are recorded by KeyJutsu alone.
6. Never put a secret, password or token in a command. Where one is needed,
   use a step of kind \"credential\" with execution_mode \"user_input\", no
   commands, and \"credential\": {\"variable\": \"NAME\", \"prompt\": \"...\",
   \"kind\": \"secret\"}; the operator types it at run time and later steps
   use $NAME.
7. Prefer commands that can be checked: built-in cmdlets over external
   programs, -LiteralPath over wildcards.
8. Never download in a command. List what a step needs as \"artifacts\"
   ({\"name\": \"tool.zip\", \"source\": \"https://...\", \"sha256\": \"...\" if
   known}); KeyJutsu downloads and checks it before the plan runs, and the
   step uses $KJ_ARTIFACTS['tool.zip']. Declare every host a step contacts in
   \"network\": {\"destinations\": [{\"host\", \"protocol\", \"purpose\",
   \"at_runtime\"}]}.
9. Say what you are unsure of in the step's reason rather than guessing.";

const COMPACT_FORMAT: &str = r#"The JSON document is a plan:
{
  "schema_version": "1.0", "plan_id": "kebab-id", "task_id": "kebab-id", "title": "...",
  "target": {"id": "local", "kind": "local_windows"},
  "agent": {"name": "<your agent name>", "version": "<your version>"},
  "environment_assumptions": [{"description": "...", "check": <condition>}],
  "steps": [{
    "id": "kebab-id", "title": "...", "objective": "...", "reason": "...",
    "kind": "command" | "validation" | "manual" | "user_input" | "credential",
    "shell": {"kind": "pwsh" | "windows_powershell" | "cmd"},
    "commands": [{"text": "one line"}],
    "depends_on": ["step-id"], "preconditions": [<condition>],
    "privilege": "standard" | "administrator",
    "proposed_risk": {"level": "low" | "normal" | "high" | "critical", "rationale": "..."},
    "reversibility": {"level": "full" | "partial" | "none", "notes": "..."},
    "expected_effects": [{"kind": "file_modified" | "service_state" | ..., "target": "..."}],
    "visible_validation": [{"text": "a command that shows success"}],
    "internal_validation": [{"exit_code": {"equals": 0}} | {"service_state": {"name": "...", "state": "running"}}],
    "recovery": {"strategy": "restore_captured_state" | "commands" | "none", "commands": [{"text": "..."}]}
  }],
  "edges": [{"from": "step-id", "to": "step-id", "when": <condition>}]
}
Without edges, steps run in the order listed. Unknown properties are refused."#;

fn format_section(full_schema: bool) -> String {
    if full_schema {
        format!(
            "The JSON document must validate against this JSON Schema (without the \"keyjutsu\" property):\n\n{PLAN_SCHEMA}"
        )
    } else {
        COMPACT_FORMAT.to_owned()
    }
}

fn context_section(context: &PreparedContext) -> String {
    let mut s = String::new();
    if let Some(dir) = &context.working_directory {
        s.push_str(&format!("You have been started in {dir}; investigate it read-only.\n\n"));
    }
    for (label, text) in &context.blocks {
        s.push_str(&format!("----- context: {label} -----\n{text}\n----- end of {label} -----\n\n"));
    }
    s
}

pub fn propose(task: &str, context: &PreparedContext, full_schema: bool) -> String {
    format!(
        "{RULES}\n\nThe operator's task:\n{task}\n\n{}Propose a plan for the task.\n\n{}",
        context_section(context),
        format_section(full_schema)
    )
}

pub fn revise_step(task: &str, plan_json: &str, step: &str, guidance: &str, findings: &[String]) -> String {
    let findings = if findings.is_empty() {
        String::from("(none)")
    } else {
        findings.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
    };
    format!(
        "{RULES}\n\nThe operator's task:\n{task}\n\nThe current plan:\n```json\n{plan_json}\n```\n\n\
         Revise only step \"{step}\". Keep its id. Do not change any other step.\n\n\
         The operator's guidance:\n{guidance}\n\nWhat KeyJutsu's validation found about this step:\n{findings}\n\n\
         Answer with the single replacement step as a JSON object (not the whole plan), in a ```json block."
    )
}

pub fn revise_plan(task: &str, plan_json: &str, guidance: &str, full_schema: bool) -> String {
    format!(
        "{RULES}\n\nThe operator's task:\n{task}\n\nThe current plan:\n```json\n{plan_json}\n```\n\n\
         Reconsider the whole plan. Keep the ids of steps you keep unchanged.\n\n\
         The operator's guidance:\n{guidance}\n\n{}",
        format_section(full_schema)
    )
}

pub fn review(task: &str, plan_json: &str) -> String {
    format!(
        "{RULES}\n\nYou are reviewing another agent's plan, not writing one. You cannot change it; \
         your findings go to the operator and the plan's author.\n\n\
         The operator's task:\n{task}\n\nThe plan:\n```json\n{plan_json}\n```\n\n\
         Look for: assumptions that may not hold, missing validation, unsafe commands, weak or missing \
         rollback, and better alternatives. Answer with a JSON document in a ```json block:\n\
         {{\"summary\": \"one paragraph\", \"findings\": [{{\"step\": \"step-id or null\", \
         \"kind\": \"assumption\" | \"missing_validation\" | \"unsafe\" | \"weak_rollback\" | \"alternative\" | \"other\", \
         \"severity\": \"info\" | \"warning\" | \"serious\", \"message\": \"...\"}}]}}"
    )
}

/// Ask again after an answer KeyJutsu could not accept.
pub fn repair(original: &str, problems: &[String]) -> String {
    let list = problems.iter().take(20).map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n");
    format!(
        "{original}\n\nYour previous answer could not be accepted. KeyJutsu reported:\n{list}\n\n\
         Answer again, correcting these, in the same format."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> PreparedContext {
        PreparedContext { manifest: Vec::new(), working_directory: None, blocks: Vec::new() }
    }

    #[test]
    fn every_prompt_carries_the_ground_rules() {
        for p in [
            propose("t", &empty(), false),
            revise_step("t", "{}", "s", "g", &[]),
            revise_plan("t", "{}", "g", false),
            review("t", "{}"),
        ] {
            assert!(p.contains("Investigate only"), "{p}");
            assert!(p.contains("Never include a \"keyjutsu\" property"));
            assert!(p.contains("no expression or script form"));
        }
    }

    #[test]
    fn the_full_schema_goes_only_where_it_fits() {
        assert!(propose("t", &empty(), true).contains("\"$defs\""));
        let compact = propose("t", &empty(), false);
        assert!(!compact.contains("\"$defs\""));
        assert!(compact.len() < crate::agents::MAX_ARGUMENT_PROMPT / 2, "{} chars", compact.len());
    }

    #[test]
    fn context_blocks_are_labelled() {
        let c = PreparedContext {
            manifest: Vec::new(),
            working_directory: Some("C:/repo".into()),
            blocks: vec![("error.log".into(), "boom".into())],
        };
        let p = propose("fix it", &c, false);
        assert!(p.contains("started in C:/repo"));
        assert!(p.contains("----- context: error.log -----\nboom\n"));
    }
}
