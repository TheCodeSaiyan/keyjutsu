// The release gates, each a named list of tests.
//
// A release must not ship if any gate fails. Every test is named exactly, and
// a test that is missing counts as a failure: renaming or deleting one cannot
// quietly take it out of a gate. The tests run once, together, in one
// `cargo test` so a gate's answer is the same run's answer.
//
//   node scripts/release-gates.mjs
import { spawnSync } from "node:child_process";

const gates = {
  "Broker security": [
    "an_approved_administrator_step_runs_and_nothing_else_does",
    "another_protocol_version_is_refused_not_negotiated",
    "a_client_without_the_secret_or_from_another_process_is_refused",
    "a_pipe_name_cannot_be_taken_over",
    "the_broker_binary_refuses_a_snapshot_it_was_not_launched_for",
    "nothing_on_the_pipe_runs_anything_but_the_approved_step_for_an_authenticated_client",
    "the_genuine_sequence_runs_the_approved_step",
    "any_byte_stream_is_read_as_frames_or_refused",
    "a_frame_claiming_four_gigabytes_is_refused_before_anything_is_allocated",
    "a_broker_that_dies_mid_step_leaves_the_step_in_doubt",
  ],
  "Plan integrity": [
    "changing_a_step_invalidates_it_and_everything_after_it",
    "hashes_and_diff_agree_on_what_a_change_affects",
    "any_edit_to_a_stored_snapshot_is_refused",
    "no_edit_to_a_snapshot_is_accepted_as_something_else",
    "a_snapshot_runs_only_if_this_account_approved_it",
    "an_approval_record_moved_to_another_snapshot_does_not_vouch_for_it",
    "run_refuses_a_snapshot_this_account_did_not_approve",
    "an_edited_snapshot_fails_verification",
    "an_edited_checkpoint_is_refused",
    "a_changed_step_runs_again_even_though_it_succeeded_before",
    "a_shared_technique_arrives_as_an_untrusted_draft",
    "an_imported_technique_cannot_claim_trust_or_hide_what_it_runs",
    "parameter_values_are_data_not_code",
  ],
  "Secret handling": [
    "a_secret_in_pasted_context_never_reaches_the_agent",
    "no_secret_reaches_an_agent_by_any_route",
    "a_failed_step_is_diagnosed_from_its_real_output",
    "fixing_a_failed_step_shows_the_agent_what_it_printed",
    "common_secret_shapes_are_removed_and_counted_but_never_echoed",
    "the_store_is_encrypted_and_records_cannot_be_swapped",
    "a_store_opened_by_many_at_once_keeps_one_key",
    "the_nonce_never_appears_in_debug_output",
    "a_credential_is_entered_and_used_without_being_seen_or_kept",
    "without_send_nothing_leaves_the_machine_and_nothing_is_written",
  ],
  "Execution state machine": [
    "mashing_arbitrary_keys_delivers_exactly_the_staged_command",
    "keys_mashed_while_a_command_runs_do_not_reach_it",
    "no_sequence_of_keys_can_submit_an_incomplete_command",
    "the_hard_disarm_chord_works_in_every_state_and_erases_partial_input",
    "steps_advance_only_when_the_shell_reports_completion",
    "a_failed_step_stops_the_performance_and_returns_control",
    "a_shell_that_exits_mid_step_is_never_a_success",
    "a_step_left_in_doubt_blocks_until_the_operator_settles_it",
    "a_step_that_cannot_be_recorded_as_starting_does_not_run",
    "the_same_snapshot_runs_in_every_mode",
    "a_mark_with_the_wrong_nonce_is_passed_through_untrusted",
    "output_without_the_nonce_is_never_a_mark_and_never_lost",
    "how_output_is_split_into_reads_does_not_matter",
    "an_unfinished_sequence_cannot_swallow_the_prompts_mark",
    "raw_input_is_refused_while_a_performance_owns_the_keyboard",
    "arming_is_refused_on_a_dirty_or_busy_line",
  ],
  "Schema validation": [
    "valid_fixtures_parse_as_proposals_and_as_plans",
    "schema_invalid_fixtures_are_refused_before_the_model_sees_them",
    "structure_invalid_fixtures_report_the_right_problem",
    "schema_messages_never_quote_the_document",
    "a_future_version_is_refused_by_name_not_guessed_at",
    "arbitrary_text_is_refused_cleanly",
    "mutated_plans_never_panic_and_always_get_the_same_answer",
  ],
  "Supported-shell compatibility": [
    "pwsh_is_a_normal_interactive_terminal",
    "mashed_keys_type_and_run_exactly_the_staged_command",
    "a_failing_command_fails_the_step_with_its_exit_code",
    "ctrl_c_interrupts_a_running_command",
    "windows_powershell_5_1_reports_marks_and_exit_codes",
    "cmd_runs_staged_commands_but_its_success_is_unverified",
    "each_shell_says_where_it_is_at_every_prompt",
    "the_safe_demo_runs_in_direct_mode",
  ],
  "Critical rollback": [
    "whole_plan_approval_never_covers_a_critical_step",
    "keyjutsus_own_critical_rating_needs_the_typed_phrase_even_if_the_agent_said_low",
    "recursive_removal_is_critical_whatever_the_agent_says",
    "a_critical_step_is_only_sealed_with_its_typed_phrase",
    "a_critical_step_is_confirmed_again_just_before_it_runs",
    "an_old_approval_of_a_critical_step_is_confirmed_again_before_the_run",
    "a_failed_reversible_task_is_recovered_without_touching_anything_else",
    "a_failed_run_is_recovered_only_when_the_operator_confirms",
    "a_backup_changed_since_it_was_taken_is_not_used",
    "a_step_whose_recovery_cannot_be_prepared_does_not_run",
  ],
  "Credential boundary": [
    "a_line_that_asks_the_operator_waits_for_enter_and_is_never_performed",
    "a_line_that_asks_the_operator_is_not_started_for_them",
    "keys_after_the_last_answer_do_not_reach_the_next_prompt",
    "a_credential_is_entered_and_used_without_being_seen_or_kept",
    "run_asks_for_a_credential_in_the_shells_masked_prompt",
    "a_resumed_run_asks_for_the_credential_again",
  ],
};

const names = [...new Set(Object.values(gates).flat())];
// The default members hold every gate's tests. The desktop crate is left out
// because building it needs the installer's staged binaries beside it.
const run = spawnSync("cargo", ["test", "--no-fail-fast", "--", ...names], {
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
  shell: false,
});
const output = `${run.stdout ?? ""}\n${run.stderr ?? ""}`;
// The names are filters, so a longer name containing one may run too; only
// a result whose last path segment is exactly the name counts for it.
// (`--exact` would compare whole paths, which unit tests in modules have.)
const results = new Map();
for (const m of output.matchAll(/^test (\S+) \.\.\. (ok|FAILED|ignored)/gm)) {
  results.set(m[1].split("::").pop(), m[2]);
}

let failed = false;
for (const [gate, tests] of Object.entries(gates)) {
  const bad = tests.filter((t) => results.get(t) !== "ok");
  failed ||= bad.length > 0;
  console.log(
    `${bad.length ? "FAIL" : "pass"}  ${gate} (${tests.length - bad.length}/${tests.length})`,
  );
  for (const t of bad) console.log(`        ${results.get(t) ?? "MISSING"}  ${t}`);
}
// A build failure leaves every test "missing"; show cargo's own words.
if (run.status !== 0 || run.error) {
  console.log(run.error ? String(run.error) : output.slice(-4000));
  console.log("cargo test itself failed; the gates cannot be trusted.");
  failed = true;
}
process.exit(failed ? 1 : 0);
