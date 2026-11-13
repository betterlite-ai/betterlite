#![cfg(test)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rstest::rstest;

use super::{Clock, ReportContext, build_report, render_report};
use crate::commands::debug::error::DebugError;
use crate::commands::debug::wire::{RequestResponsePayload, SpendLogRow};

const NOTE: &str = "Bodies are included for failed turns and the most recent turns. Bodies are empty unless the proxy runs with `general_settings.store_prompts_in_spend_logs: true`.";
const MINIMAL_SECTION: &str = "### 1. ok ?
- request_id: `a`
- time: n/a -> n/a
- provider: ? (n/a), call_type: ?
- spend: $0.000000, tokens: 0 in / 0 out";

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

type Payloads = BTreeMap<String, Option<RequestResponsePayload>>;

fn payloads(entries: &[(&str, Option<&str>)]) -> Payloads {
    entries
        .iter()
        .map(|(id, body)| {
            (
                (*id).to_owned(),
                body.map(|json| serde_json::from_str(json).unwrap()),
            )
        })
        .collect()
}

fn rows_from(json: &str) -> Vec<SpendLogRow> {
    serde_json::from_str(json).unwrap()
}

fn render_at(clock: &FixedClock, rows: &str, bodies: &Payloads, max_body_chars: usize) -> String {
    let context = ReportContext {
        session_id: "s1",
        base_url: "http://proxy",
        max_body_chars,
        clock,
    };
    render_report(&context, &rows_from(rows), bodies).unwrap()
}

fn report(rows: &str) -> String {
    let clock = FixedClock(DateTime::from_timestamp(1_767_225_600, 0).unwrap());
    render_at(&clock, rows, &Payloads::new(), 20_000)
}

fn report_with_bodies(rows: &str, bodies: &Payloads, max_body_chars: usize) -> String {
    let clock = FixedClock(DateTime::from_timestamp(1_767_225_600, 0).unwrap());
    render_at(&clock, rows, bodies, max_body_chars)
}

fn after_turns_heading(report: &str) -> &str {
    report.split_once("## Turns\n").unwrap().1
}

#[test]
fn render_writes_the_whole_report_of_one_turn() {
    let rendered = report(
        r#"[{"request_id":"r1","model":"claude","startTime":"t1","endTime":"t2","custom_llm_provider":"anthropic","api_base":"http://upstream","call_type":"acompletion","spend":0.5,"prompt_tokens":3,"completion_tokens":4,"total_tokens":99}]"#,
    );

    assert_eq!(
        rendered,
        format!(
            "# LiteLLM debug report: Claude Code session `s1`

- proxy: http://proxy
- generated: 2026-01-01T00:00:00+00:00
- turns: 1, failed: 0
- total spend: $0.500000
- models: claude

{NOTE}

## Turns
### 1. ok claude
- request_id: `r1`
- time: t1 -> t2
- provider: anthropic (http://upstream), call_type: acompletion
- spend: $0.500000, tokens: 3 in / 4 out
"
        )
    );
}

#[test]
fn render_fills_absent_fields_with_placeholders() {
    let rendered = report(r#"[{"request_id":"a"}]"#);

    assert_eq!(
        after_turns_heading(&rendered),
        format!("{MINIMAL_SECTION}\n")
    );
}

#[test]
fn render_separates_turns_with_one_blank_line_and_ends_with_one_newline() {
    let rendered = report(r#"[{"request_id":"a"},{"request_id":"b"}]"#);

    assert_eq!(
        after_turns_heading(&rendered),
        format!(
            "{MINIMAL_SECTION}

### 2. ok ?
- request_id: `b`
- time: n/a -> n/a
- provider: ? (n/a), call_type: ?
- spend: $0.000000, tokens: 0 in / 0 out
"
        )
    );
}

#[test]
fn render_numbers_the_turns_from_one() {
    let rendered = report(r#"[{"request_id":"a"},{"request_id":"b"},{"request_id":"c"}]"#);

    let headers: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with("### "))
        .collect();
    assert_eq!(headers, ["### 1. ok ?", "### 2. ok ?", "### 3. ok ?"]);
}

#[test]
fn render_counts_turns_and_failures() {
    let rendered = report(
        r#"[{"request_id":"a","status":"failure"},{"request_id":"b"},{"request_id":"c","metadata":{"status":"failure"}}]"#,
    );

    assert!(rendered.contains("\n- turns: 3, failed: 2\n"));
}

#[rstest]
#[case::sum_of_rows(
    r#"[{"request_id":"a","spend":0.1},{"request_id":"b","spend":0.2}]"#,
    "$0.300000"
)]
#[case::no_spend(r#"[{"request_id":"a"}]"#, "$0.000000")]
#[case::rounds_a_midpoint_up(r#"[{"request_id":"a","spend":0.0000005}]"#, "$0.000001")]
#[case::rounds_below_the_midpoint_down(r#"[{"request_id":"a","spend":0.0000004}]"#, "$0.000000")]
#[case::keeps_six_places(r#"[{"request_id":"a","spend":1.5e-05}]"#, "$0.000015")]
#[case::whole_number(r#"[{"request_id":"a","spend":12}]"#, "$12.000000")]
#[case::sums_before_rounding(
    r#"[{"request_id":"a","spend":0.0000004},{"request_id":"b","spend":0.0000004},{"request_id":"c","spend":0.0000004}]"#,
    "$0.000001"
)]
fn render_prints_the_total_spend_with_six_decimals(#[case] rows: &str, #[case] total: &str) {
    let rendered = report(rows);

    assert!(rendered.contains(&format!("\n- total spend: {total}\n")));
}

#[rstest]
#[case::midpoint_up("0.0000005", "$0.000001")]
#[case::below_the_midpoint("0.0000004", "$0.000000")]
#[case::exponent("1.5e-05", "$0.000015")]
#[case::beyond_six_places("0.1234567", "$0.123457")]
fn render_prints_the_spend_of_a_turn_with_six_decimals(#[case] spend: &str, #[case] shown: &str) {
    let rendered = report(&format!(r#"[{{"request_id":"a","spend":{spend}}}]"#));

    assert!(rendered.contains(&format!("\n- spend: {shown}, tokens: 0 in / 0 out")));
}

#[test]
fn render_reports_a_total_that_does_not_fit_a_decimal() {
    let clock = FixedClock(DateTime::from_timestamp(0, 0).unwrap());
    let context = ReportContext {
        session_id: "s1",
        base_url: "http://proxy",
        max_body_chars: 10,
        clock: &clock,
    };
    let rows = rows_from(
        r#"[{"request_id":"a","spend":79228162514264337593543950335},{"request_id":"b","spend":1}]"#,
    );

    let error = render_report(&context, &rows, &Payloads::new()).unwrap_err();

    assert!(matches!(error, DebugError::SpendOverflow));
}

#[test]
fn render_lists_the_sorted_distinct_models() {
    let rendered = report(
        r#"[{"request_id":"a","model":"zeta"},{"request_id":"b","model":"alpha"},{"request_id":"c","model":"zeta"},{"request_id":"d","model_group":"mid"},{"request_id":"e"}]"#,
    );

    assert!(rendered.contains("\n- models: ?, alpha, mid, zeta\n"));
}

#[test]
fn render_lists_n_a_when_there_are_no_turns() {
    let rendered = report("[]");

    assert!(
        rendered.contains("\n- turns: 0, failed: 0\n- total spend: $0.000000\n- models: n/a\n")
    );
}

#[rstest]
#[case::whole_seconds(0, "2026-01-01T00:00:00+00:00")]
#[case::fraction_is_dropped(999_999_999, "2026-01-01T00:00:00+00:00")]
fn render_stamps_the_report_in_utc_to_the_second(#[case] nanos: u32, #[case] expected: &str) {
    let clock = FixedClock(DateTime::from_timestamp(1_767_225_600, nanos).unwrap());

    let rendered = render_at(&clock, "[]", &Payloads::new(), 10);

    assert!(rendered.contains(&format!("\n- generated: {expected}\n")));
}

#[test]
fn render_explains_when_the_bodies_are_empty() {
    let rendered = report("[]");

    assert!(rendered.contains(&format!("\n\n{NOTE}\n\n## Turns\n")));
}

#[rstest]
#[case::own_failure(r#","status":"failure""#, "### 1. FAILED m")]
#[case::metadata_failure(r#","metadata":{"status":"failure"}"#, "### 1. FAILED m")]
#[case::success(r#","status":"success""#, "### 1. ok m")]
#[case::success_over_metadata(
    r#","status":"success","metadata":{"status":"failure"}"#,
    "### 1. ok m"
)]
#[case::no_status("", "### 1. ok m")]
fn render_marks_failed_turns(#[case] fields: &str, #[case] header: &str) {
    let rendered = report(&format!(r#"[{{"request_id":"a","model":"m"{fields}}}]"#));

    assert_eq!(after_turns_heading(&rendered).lines().next(), Some(header));
}

#[test]
fn render_falls_back_to_the_model_group_in_the_header() {
    let rendered = report(r#"[{"request_id":"a","model_group":"group"}]"#);

    assert!(after_turns_heading(&rendered).starts_with("### 1. ok group\n"));
}

#[rstest]
#[case::everything(
    r#"{"error_code":"429","error_class":"RateLimit","error_message":"slow down"}"#,
    "- error: `429` RateLimit\n\n```\nslow down\n```"
)]
#[case::no_class(
    r#"{"error_code":"429","error_message":"slow down"}"#,
    "- error: `429`\n\n```\nslow down\n```"
)]
#[case::no_code(
    r#"{"error_class":"RateLimit","error_message":"slow down"}"#,
    "- error: `?` RateLimit\n\n```\nslow down\n```"
)]
#[case::no_message(
    r#"{"error_code":"500","error_class":"Boom"}"#,
    "- error: `500` Boom\n\n```\n\n```"
)]
#[case::nothing("{}", "- error: `?`\n\n```\n\n```")]
fn render_prints_the_error_of_a_failed_turn(#[case] information: &str, #[case] block: &str) {
    let rendered = report(&format!(
        r#"[{{"request_id":"a","status":"failure","metadata":{{"error_information":{information}}}}}]"#
    ));

    let expected = format!(
        "### 1. ok ?
- request_id: `a`
- time: n/a -> n/a
- provider: ? (n/a), call_type: ?
- spend: $0.000000, tokens: 0 in / 0 out
{block}
"
    )
    .replacen("ok ?", "FAILED ?", 1);
    assert_eq!(after_turns_heading(&rendered), expected);
}

#[rstest]
#[case::success_with_error_information(
    r#","status":"success","metadata":{"error_information":{"error_code":"1","error_message":"m"}}"#
)]
#[case::no_status_with_error_information(
    r#","metadata":{"error_information":{"error_code":"1","error_message":"m"}}"#
)]
#[case::failure_without_error_information(r#","status":"failure""#)]
#[case::failure_with_empty_metadata(r#","status":"failure","metadata":{}"#)]
fn render_prints_no_error_block_without_a_failure_and_its_information(#[case] fields: &str) {
    let rendered = report(&format!(r#"[{{"request_id":"a"{fields}}}]"#));

    assert!(!rendered.contains("- error:"));
}

#[rstest]
#[case::no_backticks("plain", "```")]
#[case::empty("", "```")]
#[case::one("a`b", "```")]
#[case::two("a``b", "```")]
#[case::three("a```b", "````")]
#[case::four("````", "`````")]
#[case::longest_of_several("`` ````` `", "``````")]
fn render_fences_an_error_message_longer_than_any_backtick_run_inside(
    #[case] message: &str,
    #[case] fence: &str,
) {
    let rendered = report(&format!(
        r#"[{{"request_id":"a","status":"failure","metadata":{{"error_information":{{"error_message":"{message}"}}}}}}]"#
    ));

    assert!(rendered.contains(&format!("\n\n{fence}\n{message}\n{fence}\n")));
}

#[test]
fn render_keeps_the_lines_of_an_error_message() {
    let rendered = report(
        r#"[{"request_id":"a","status":"failure","metadata":{"error_information":{"error_message":"one\ntwo"}}}]"#,
    );

    assert!(rendered.ends_with("\n```\none\ntwo\n```\n"));
}

#[test]
fn render_adds_the_request_and_response_of_a_turn_that_has_a_payload() {
    let bodies = payloads(&[(
        "a",
        Some(r#"{"proxy_server_request":{"k":1},"response":"done"}"#),
    )]);

    let rendered = report_with_bodies(r#"[{"request_id":"a"}]"#, &bodies, 20_000);

    assert_eq!(
        after_turns_heading(&rendered),
        format!(
            "{MINIMAL_SECTION}

<details><summary>request body</summary>

```json
{{
  \"k\": 1
}}
```
</details>

<details><summary>response</summary>

```json
done
```
</details>
"
        )
    );
}

#[rstest]
#[case::no_entry(&[])]
#[case::entry_without_a_payload(&[("a", None)])]
#[case::payload_of_another_request(&[("b", Some("{}"))])]
fn render_adds_no_bodies_without_a_payload_for_the_turn(#[case] entries: &[(&str, Option<&str>)]) {
    let rendered = report_with_bodies(r#"[{"request_id":"a"}]"#, &payloads(entries), 20_000);

    assert_eq!(
        after_turns_heading(&rendered),
        format!("{MINIMAL_SECTION}\n")
    );
}

#[test]
fn render_adds_bodies_only_to_the_turns_that_have_one() {
    let bodies = payloads(&[("b", Some(r#"{"response":"r"}"#))]);

    let rendered = report_with_bodies(
        r#"[{"request_id":"a"},{"request_id":"b"},{"request_id":"c"}]"#,
        &bodies,
        20_000,
    );

    assert_eq!(rendered.matches("<summary>response</summary>").count(), 1);
    let second = rendered.split("### 2.").nth(1).unwrap();
    assert!(second.split("### 3.").next().unwrap().contains("<details>"));
}

#[test]
fn render_puts_the_bodies_after_the_error_of_a_failed_turn() {
    let bodies = payloads(&[("a", Some(r#"{"response":"r"}"#))]);

    let rendered = report_with_bodies(
        r#"[{"request_id":"a","status":"failure","metadata":{"error_information":{"error_code":"1","error_message":"m"}}}]"#,
        &bodies,
        20_000,
    );

    let error_at = rendered.find("- error:").unwrap();
    let bodies_at = rendered.find("<details>").unwrap();
    assert!(error_at < bodies_at);
}

#[rstest]
#[case::kept_whole(6, "abcdef")]
#[case::one_over(5, "abcde\n... (truncated, 1 more chars)")]
#[case::all_but_one(1, "a\n... (truncated, 5 more chars)")]
#[case::zero(0, "\n... (truncated, 6 more chars)")]
#[case::roomy(100, "abcdef")]
fn render_truncates_each_body_to_the_character_limit(#[case] limit: usize, #[case] expected: &str) {
    let bodies = payloads(&[(
        "a",
        Some(r#"{"proxy_server_request":"abcdef","response":"abcdef"}"#),
    )]);

    let rendered = report_with_bodies(r#"[{"request_id":"a"}]"#, &bodies, limit);

    let block = format!("```json\n{expected}\n```\n</details>");
    assert_eq!(rendered.matches(&block).count(), 2);
}

#[test]
fn render_counts_characters_and_not_bytes_when_truncating() {
    let bodies = payloads(&[(
        "a",
        Some(r#"{"proxy_server_request":"éééé","response":"x"}"#),
    )]);

    let rendered = report_with_bodies(r#"[{"request_id":"a"}]"#, &bodies, 3);

    assert!(rendered.contains("```json\n\u{e9}\u{e9}\u{e9}\n... (truncated, 1 more chars)\n```"));
}

#[test]
fn render_fences_a_body_longer_than_any_backtick_run_inside() {
    let bodies = payloads(&[(
        "a",
        Some(r#"{"proxy_server_request":"a```b","response":"x"}"#),
    )]);

    let rendered = report_with_bodies(r#"[{"request_id":"a"}]"#, &bodies, 20_000);

    assert!(rendered.contains("\n````json\na```b\n````\n</details>"));
}

#[test]
fn render_refuses_a_report_without_turns() {
    let clock = FixedClock(DateTime::from_timestamp(0, 0).unwrap());
    let context = ReportContext {
        session_id: "s9",
        base_url: "http://proxy.example",
        max_body_chars: 10,
        clock: &clock,
    };

    let error = build_report(&context, &[], &Payloads::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "No spend logs found for session 's9' on http://proxy.example. Is Claude Code routed through this proxy (`lite up`), and does your key have log access?"
    );
}

#[test]
fn render_builds_the_report_of_a_session_with_turns() {
    let clock = FixedClock(DateTime::from_timestamp(1_767_225_600, 0).unwrap());
    let context = ReportContext {
        session_id: "s1",
        base_url: "http://proxy",
        max_body_chars: 10,
        clock: &clock,
    };

    let built = build_report(
        &context,
        &rows_from(r#"[{"request_id":"a"}]"#),
        &Payloads::new(),
    )
    .unwrap();

    assert_eq!(built, report(r#"[{"request_id":"a"}]"#));
}
