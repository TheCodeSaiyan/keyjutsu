//! Getting the answer out of what an agent printed.
//!
//! Agents wrap their final message differently: Claude Code's and Gemini's
//! JSON output put it in an envelope, Codex writes it to a file, Copilot
//! prints it as text. Inside the message, the answer is a fenced JSON block,
//! or failing that the first JSON object that parses. Everything found here is
//! untrusted and goes through the same gates as any proposal.

use serde_json::Value;

/// The agent's final message, from its raw output.
pub fn final_message(stdout: &str, output_file: Option<&str>) -> String {
    if let Some(file) = output_file
        && !file.trim().is_empty()
    {
        return file.to_owned();
    }
    let trimmed = stdout.trim();
    if let Ok(Value::Object(envelope)) = serde_json::from_str::<Value>(trimmed) {
        for key in ["result", "response", "text", "content"] {
            if let Some(Value::String(s)) = envelope.get(key) {
                return s.clone();
            }
        }
    }
    trimmed.to_owned()
}

/// Whether an envelope reports the agent itself failing, and why.
pub fn envelope_error(stdout: &str) -> Option<String> {
    let Ok(Value::Object(envelope)) = serde_json::from_str::<Value>(stdout.trim()) else { return None };
    if envelope.get("is_error").and_then(Value::as_bool) == Some(true) {
        let why = envelope.get("result").and_then(Value::as_str).unwrap_or("the agent reported an error");
        return Some(why.chars().take(300).collect());
    }
    envelope.get("error").map(|e| e.to_string().chars().take(300).collect())
}

/// The JSON document in `message`: the last ```json block, else the last
/// fenced block that parses, else the first `{` that starts a document that
/// parses.
pub fn json_document(message: &str) -> Option<Value> {
    let mut fenced = Vec::new();
    let mut rest = message;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let (lang, body_start) = match after.find('\n') {
            Some(nl) => (after[..nl].trim(), nl + 1),
            None => break,
        };
        let body = &after[body_start..];
        let Some(end) = body.find("```") else { break };
        fenced.push((lang.to_ascii_lowercase(), &body[..end]));
        rest = &body[end + 3..];
    }
    let parse = |s: &str| serde_json::from_str::<Value>(s.trim()).ok().filter(Value::is_object);
    if let Some(v) = fenced.iter().rev().filter(|(l, _)| l == "json").find_map(|(_, b)| parse(b)) {
        return Some(v);
    }
    if let Some(v) = fenced.iter().rev().find_map(|(_, b)| parse(b)) {
        return Some(v);
    }
    // No usable fence: try each `{` in turn with a streaming parser, which
    // stops at the end of the first complete value.
    message.char_indices().filter(|(_, c)| *c == '{').find_map(|(i, _)| {
        let mut stream = serde_json::Deserializer::from_str(&message[i..]).into_iter::<Value>();
        stream.next()?.ok().filter(Value::is_object)
    })
}

/// The text around the JSON, for the operator: the agent's own summary.
pub fn summary(message: &str) -> String {
    match message.rfind("```") {
        Some(end) => message[end + 3..].trim().chars().take(2000).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_each_agents_envelope() {
        // Shapes as the CLIs document them: Claude Code's --output-format json
        // and Gemini's -o json put the final message in a field.
        let claude = r#"{"type":"result","subtype":"success","is_error":false,"result":"hello from claude","session_id":"x"}"#;
        assert_eq!(final_message(claude, None), "hello from claude");
        let gemini = r#"{"response":"hello from gemini","stats":{}}"#;
        assert_eq!(final_message(gemini, None), "hello from gemini");
        assert_eq!(final_message("plain text\n", None), "plain text");
        assert_eq!(final_message("ignored", Some("from the file")), "from the file");
    }

    #[test]
    fn reports_an_agent_that_failed_inside_its_envelope() {
        let err = r#"{"type":"result","is_error":true,"result":"Credit balance is too low"}"#;
        assert_eq!(envelope_error(err).as_deref(), Some("Credit balance is too low"));
        assert_eq!(envelope_error(r#"{"is_error":false,"result":"ok"}"#), None);
    }

    #[test]
    fn prefers_the_last_json_fence_and_ignores_other_code() {
        let message = "I looked around.\n```powershell\nGet-Service\n```\nDraft:\n```json\n{\"a\":1}\n```\nFinal:\n```json\n{\"a\":2}\n```\nThat is my plan.";
        assert_eq!(json_document(message), Some(serde_json::json!({"a": 2})));
        assert_eq!(summary(message), "That is my plan.");
    }

    #[test]
    fn finds_bare_json_when_there_is_no_fence() {
        let message = "Here you go: {\"plan\": {\"steps\": []}} and some trailing words {not json";
        assert_eq!(json_document(message), Some(serde_json::json!({"plan": {"steps": []}})));
        assert_eq!(json_document("no json at all"), None);
        assert_eq!(json_document("```json\n[1, 2]\n```"), None, "a document is an object");
    }
}
