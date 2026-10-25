#![cfg(test)]

use std::fs;
use std::path::Path;
use std::str;

use rstest::rstest;

use crate::cache_plan::{
    CacheBoundary, PromptCachePlan, UnsupportedCachePlan, cache_scope, parse_cache_plan,
    parse_prompt,
};

const AUTOMATIC: &str = r#"{"model":"claude","max_tokens":32,"cache_control":{"type":"ephemeral"},"messages":[{"role":"user","content":"hello"}]}"#;
const SETTINGS: &str = r#"{"thinking":{"type":"enabled","budget_tokens":16},"output_config":{"effort":"low"},"speed":"fast","tool_choice":{"type":"auto"},"tools":[{"name":"lookup","input_schema":{"type":"object","properties":{"x":{"minimum":1e-05,"maximum":1.5e16,"default":0.5,"big":1E2,"tiny":0.0001,"wide":1e15,"small":-1.5e-7}}}}],"system":"caf\u00e9 \u001c\"\\","messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"},{"input":{"b":1,"a":-0.0},"name":"lookup","id":"call","type":"tool_use","cache_control":{"type":"ephemeral"}}]}]}"#;
const TOOL_GROUPS: &str = r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a"},{"type":"tool_use","id":"one","name":"lookup","input":{}},{"type":"tool_use","id":"two","name":"lookup","input":{}},{"type":"tool_result","tool_use_id":"one","content":"done"},{"type":"tool_result","tool_use_id":"two","content":[{"type":"text","text":""}],"is_error":false,"cache_control":{"type":"ephemeral","ttl":"1h"}}]}]}"#;
const MARKED: &str = r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#;
const HIERARCHY: &str = r#"{"tools":[{"name":"lookup","input_schema":{"type":"object"},"cache_control":{"type":"ephemeral","ttl":"1h"}}],"system":[{"type":"text","text":"Stable instructions","cache_control":{"type":"ephemeral","ttl":"1h"}}],"messages":[{"role":"user","content":[{"type":"text","text":"First turn","cache_control":{"type":"ephemeral","ttl":"5m"}}]}]}"#;

fn plan(body: &str) -> PromptCachePlan {
    parse_cache_plan(body).unwrap()
}

fn only(body: &str) -> CacheBoundary {
    let [boundary] = <[CacheBoundary; 1]>::try_from(plan(body).breakpoints).unwrap();
    boundary
}

#[rstest]
#[case::automatic_cache(
    AUTOMATIC,
    "d0e92f72a94f6117431eead0fc730184132a1099a1628a4d9c2469b5bbacaef1",
    "f351cd61c9e1e5b0d21836f81c20f13c09d97e3e39cd175e8d36cbaf2e1080fb"
)]
#[case::settings_floats_and_escapes(
    SETTINGS,
    "663540eeb52318119c26e344c298067bd38df30af0b40e08dc3067067b18c09c",
    "23869285f4366841f5ff445d75e42756920731ebdba48ac7741110092aa96ab8"
)]
#[case::grouped_tool_blocks(
    TOOL_GROUPS,
    "56690334862da2e55c411dd3421286e5b39b07ccc2a51a95efd9d4765414d88d",
    "1280e0382772a3ca6e54346013d2a6c40d9039af0af8c3797544d0798756ad86"
)]
fn fingerprints_match_litellm(
    #[case] body: &str,
    #[case] fingerprint: &str,
    #[case] content: &str,
) {
    let boundary = only(body);
    assert_eq!(boundary.fingerprint, fingerprint);
    assert_eq!(boundary.content_fingerprint, content);
}

#[test]
fn lookback_walks_positions_newest_first() {
    let boundary = only(TOOL_GROUPS);
    assert_eq!(boundary.ttl_seconds, 3600);
    assert_eq!(
        boundary.lookback_content_fingerprints,
        [
            "1280e0382772a3ca6e54346013d2a6c40d9039af0af8c3797544d0798756ad86",
            "bf26ea65c810c8351e5f608c9637dac2d87abd4c606f14b874e4aded2876ec0f",
            "2922f2609b0feb9bff50895d1496aa575071310097052534a863b060d5692fdb",
            "32c1867b60fe9901e67b82a81f4c46589c82635a47c43eeb9c510cb6130387ad",
            "c3974c054bc68596f56afb4a9bb6e93d05e930e12cc7817bb5b064c7984eeb31",
        ]
    );
    assert_eq!(
        boundary.lookback_fingerprints,
        [
            "56690334862da2e55c411dd3421286e5b39b07ccc2a51a95efd9d4765414d88d",
            "1929512f960dbf6e2214f29d7a3d7a65e4b8b3a5a76aeffa30fd0a2edd371b93",
            "28d69fa412479afb12d0d500f07a2d174d6dbd7f99f810da271803ec7a1732fa",
            "9f901c5337077f3f4222b74d12ce9ad0fda20e072416dfade38ada5e113c20aa",
            "e5505a3baa2d253dc8a09d14064db4b618df4891758a3c549fda98344f299cbd",
        ]
    );
}

#[test]
fn prefix_body_keeps_options_sections_and_key_order() {
    let expected = concat!(
        r#"{"thinking":{"type":"enabled","budget_tokens":16},"tool_choice":{"type":"auto"},"output_config":{"effort":"low"},"#,
        r#""tools":[{"name":"lookup","input_schema":{"type":"object","properties":{"x":{"minimum":1e-05,"maximum":1.5e+16,"default":0.5,"big":100.0,"tiny":0.0001,"wide":1000000000000000.0,"small":-1.5e-07}}}}],"#,
        r#""system":[{"type":"text","text":"caf"#,
        "\u{e9}",
        r#" \u001c\"\\"}],"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"},{"input":{"b":1,"a":-0.0},"name":"lookup","id":"call","type":"tool_use"}]}]}"#,
    );
    let boundary = only(SETTINGS);
    assert_eq!(boundary.prefix_body.get(), expected);
    assert_eq!(
        boundary.lookback_content_fingerprints,
        [
            "23869285f4366841f5ff445d75e42756920731ebdba48ac7741110092aa96ab8",
            "a89825f65e69b62b4a8d7bb892991bb8246a6f1a59c76feb69f4ba75d95c0969",
            "b64d33018d66efcaf1f257b7e2029e110bafbf31f9e549b4bdda550e9b50c334",
            "7959b7a5a17447d8ed3aceb92b20853be882b179a784ad89405290cb01c77e65",
        ]
    );
}

#[rstest]
#[case::thinking_is_never_automatic(
    r#"{"cache_control":{"type":"ephemeral","ttl":"1h"},"thinking":{"type":"adaptive"},"messages":[{"role":"assistant","content":[{"type":"text","text":"A stable answer"},{"type":"thinking","thinking":"Thinking","signature":"signature"}]}]}"#,
    r#"{"thinking":{"type":"adaptive"},"messages":[{"role":"assistant","content":[{"type":"text","text":"A stable answer"}]}]}"#
)]
#[case::python_blank_text_is_never_automatic(
    r#"{"cache_control":{"type":"ephemeral"},"messages":[{"role":"user","content":[{"type":"text","text":"keep"},{"type":"text","text":"\u001c"}]}]}"#,
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"keep"}]}]}"#
)]
#[case::messages_group_by_index_and_skip_empty(
    r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":[]},{"role":"user","content":[{"type":"text","text":"b"},{"type":"text","text":"c","cache_control":{"type":"ephemeral"}}]}]}"#,
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a"}]},{"role":"user","content":[{"type":"text","text":"b"},{"type":"text","text":"c"}]}]}"#
)]
#[case::tool_marker_keeps_tool_fields(
    r#"{"tools":[{"name":"t","description":null,"input_schema":{},"type":"custom","cache_control":{"type":"ephemeral","ttl":"1h"}}],"messages":[{"role":"user","content":"hi"}]}"#,
    r#"{"tools":[{"name":"t","description":null,"input_schema":{},"type":"custom"}],"messages":[]}"#
)]
#[case::tool_result_blocks(
    r#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":[{"type":"text","text":" "}],"is_error":true,"cache_control":{"type":"ephemeral"}}]}]}"#,
    r#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":[{"type":"text","text":" "}],"is_error":true}]}]}"#
)]
#[case::request_options_stay_out(
    r#"{"model":"m","max_tokens":1,"stream":true,"temperature":0.5,"top_p":1,"top_k":3,"stop_sequences":["x"],"metadata":{"user_id":"u"},"speed":"standard","service_tier":"standard_only","messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#,
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}"#
)]
fn single_breakpoint_prefix_matches_litellm(#[case] body: &str, #[case] prefix: &str) {
    assert_eq!(only(body).prefix_body.get(), prefix);
}

#[rstest]
#[case::nested_duplicate_keys_stay(r#"{"q":1,"q":2}"#, r#"{"q":1,"q":2}"#)]
#[case::integer_past_64_bits_is_a_float(
    r#"{"q":18446744073709551616}"#,
    r#"{"q":1.8446744073709552e+19}"#
)]
fn nested_json_deviates_from_python(#[case] input: &str, #[case] hashed: &str) {
    let block =
        |input: &str| format!(r#"{{"type":"tool_use","id":"a","name":"t","input":{input}}}"#);
    let body = format!(
        r#"{{"messages":[{{"role":"assistant","content":[{},{{"type":"text","text":"a","cache_control":{{"type":"ephemeral"}}}}]}}]}}"#,
        block(input)
    );
    assert_eq!(
        only(&body).prefix_body.get(),
        format!(
            r#"{{"messages":[{{"role":"assistant","content":[{},{{"type":"text","text":"a"}}]}}]}}"#,
            block(hashed)
        )
    );
}

#[rstest]
#[case::system_role_and_empty_content(
    r#"{"messages":[{"role":"system","content":""},{"role":"user","content":[]}]}"#
)]
#[case::every_option_null(
    r#"{"model":null,"max_tokens":null,"stream":null,"temperature":null,"top_p":null,"top_k":null,"stop_sequences":null,"metadata":null,"system":null,"tools":null,"cache_control":null,"thinking":null,"tool_choice":null,"output_config":null,"speed":null,"service_tier":null,"messages":[{"role":"user","content":"hi"}]}"#
)]
fn unmarked_plans_have_no_breakpoints(#[case] body: &str) {
    assert!(plan(body).breakpoints.is_empty());
}

#[test]
fn hierarchical_prefixes_share_earlier_sections() {
    let before = plan(HIERARCHY);
    let after = plan(&HIERARCHY.replace("Stable instructions", "Changed instructions"));
    let ttls: Vec<u32> = before
        .breakpoints
        .iter()
        .map(|item| item.ttl_seconds)
        .collect();
    assert_eq!(ttls, [3600, 3600, 300]);
    assert_eq!(
        before.breakpoints[0].prefix_body.get(),
        r#"{"tools":[{"name":"lookup","input_schema":{"type":"object"}}],"messages":[]}"#
    );
    assert_eq!(
        before.breakpoints[0].fingerprint,
        after.breakpoints[0].fingerprint
    );
    assert_ne!(
        before.breakpoints[1].fingerprint,
        after.breakpoints[1].fingerprint
    );
    assert_ne!(
        before.breakpoints[2].fingerprint,
        after.breakpoints[2].fingerprint
    );
    assert_eq!(before.full_body.get(), HIERARCHY);
}

#[rstest]
#[case::text(r#"{"type":"text","text":"Added"}"#, 19, true)]
#[case::text_past_twenty_groups(r#"{"type":"text","text":"Added"}"#, 20, false)]
#[case::tool_use_groups(r#"{"type":"tool_use","id":"t","name":"lookup","input":{}}"#, 30, true)]
#[case::tool_result_groups(
    r#"{"type":"tool_result","tool_use_id":"t","content":"done"}"#,
    30,
    true
)]
fn lookback_spans_twenty_groups(#[case] block: &str, #[case] added: usize, #[case] found: bool) {
    let previous = only(MARKED);
    let mut blocks = vec![r#"{"type":"text","text":"a"}"#.to_owned()];
    blocks.extend(vec![block.to_owned(); added.saturating_sub(1)]);
    blocks.push(format!(
        r#"{},"cache_control":{{"type":"ephemeral"}}}}"#,
        block.strip_suffix('}').unwrap()
    ));
    let joined = blocks.join(",");
    let current = only(&format!(
        r#"{{"messages":[{{"role":"user","content":[{joined}]}}]}}"#
    ));
    assert_eq!(
        current
            .lookback_fingerprints
            .contains(&previous.fingerprint),
        found
    );
}

const TOOL_TURN: &str = r#"{"output_config":{"effort":"low"},"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{"a":1,"b":2,"cache_control":{"ttl":"user-data"}},"cache_control":{"type":"ephemeral","ttl":"5m"}}]}]}"#;

#[rstest]
#[case::tool_input_order(r#""input":{"a":1,"b":2,"#, r#""input":{"b":2,"a":1,"#, false, false)]
#[case::effort(r#""low""#, r#""high""#, false, false)]
#[case::standard_speed(r#"{"output"#, r#"{"speed":"standard","output"#, true, true)]
#[case::fast_speed(r#"{"output"#, r#"{"speed":"fast","output"#, false, false)]
#[case::null_speed(r#"{"output"#, r#"{"speed":null,"output"#, false, false)]
#[case::tool_choice(
    r#"{"output"#,
    r#"{"tool_choice":{"type":"auto"},"output"#,
    false,
    false
)]
#[case::ttl(r#""5m""#, r#""1h""#, false, true)]
fn identity_follows_settings_and_content(
    #[case] from: &str,
    #[case] to: &str,
    #[case] same_prefix: bool,
    #[case] same_content: bool,
) {
    let before = only(TOOL_TURN);
    let after = only(&TOOL_TURN.replacen(from, to, 1));
    assert!(before.prefix_body.get().contains("user-data"));
    assert_eq!(before.fingerprint == after.fingerprint, same_prefix);
    assert_eq!(
        after.lookback_fingerprints.contains(&before.fingerprint),
        same_prefix
    );
    assert_eq!(
        before.content_fingerprint == after.content_fingerprint,
        same_content
    );
    assert_eq!(
        after
            .lookback_content_fingerprints
            .contains(&before.content_fingerprint),
        same_content
    );
}

#[rstest]
#[case::not_json("{", UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::not_an_object("[]", UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::unknown_setting(
    r#"{"unknown":true,"messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::image_block(
    r#"{"messages":[{"role":"user","content":[{"type":"image"}]}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::no_messages(r#"{"messages":[]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::blank_text(
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":" \t"}]}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::empty_tool_name(
    r#"{"tools":[{"name":"","input_schema":{}}],"messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::empty_tool_use_id(r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"","name":"t","input":{}}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::empty_tool_result_id(r#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"","content":"x"}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::unsigned_thinking(r#"{"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":""}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::unknown_ttl(r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"2h"}}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::null_ttl(r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":null}}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::persistent_cache(r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"persistent"}}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::float_max_tokens(
    r#"{"max_tokens":32.0,"messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::integer_stream(
    r#"{"stream":1,"messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::boolean_temperature(
    r#"{"temperature":true,"messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::tool_input_list(r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"t","input":[]}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::unknown_service_tier(
    r#"{"service_tier":"priority","messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::marked_result_text(r#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":[{"type":"text","text":"x","cache_control":{"type":"ephemeral"}}]}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::lone_surrogate(r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"t","input":{"q":"\ud800"},"cache_control":{"type":"ephemeral"}}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::not_a_number(r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"t","input":{"q":NaN}}]}]}"#, UnsupportedCachePlan::UnsupportedPromptShape)]
#[case::max_tokens_past_64_bits(
    r#"{"max_tokens":18446744073709551616,"messages":[{"role":"user","content":"hi"}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::duplicate_key(
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","type":"text"}]}]}"#,
    UnsupportedCachePlan::UnsupportedPromptShape
)]
#[case::conflicting_ttl(r#"{"cache_control":{"type":"ephemeral","ttl":"1h"},"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, UnsupportedCachePlan::ConflictingCacheTtl)]
#[case::reversed_ttl_order(r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]}"#, UnsupportedCachePlan::InvalidCacheTtlOrder)]
#[case::automatic_fifth_breakpoint(r#"{"cache_control":{"type":"ephemeral"},"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b","cache_control":{"type":"ephemeral"}},{"type":"text","text":"c","cache_control":{"type":"ephemeral"}},{"type":"text","text":"d","cache_control":{"type":"ephemeral"}},{"type":"text","text":"e"}]}]}"#, UnsupportedCachePlan::TooManyCacheBreakpoints)]
fn unsupported_plans_name_their_reason(#[case] body: &str, #[case] reason: UnsupportedCachePlan) {
    assert_eq!(parse_cache_plan(body).unwrap_err(), reason);
}

#[test]
fn matching_automatic_marker_adds_no_breakpoint() {
    let body = MARKED.replace(
        r#"{"messages""#,
        r#"{"cache_control":{"type":"ephemeral","ttl":"5m"},"messages""#,
    );
    assert_eq!(only(&body).fingerprint, only(MARKED).fingerprint);
}

#[rstest]
#[case::one_marker(MARKED, true)]
#[case::string_message(r#"{"messages":[{"role":"user","content":"Context"},{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, true)]
#[case::plain_system_and_tools(r#"{"system":[{"type":"text","text":"s"}],"tools":[{"name":"t","input_schema":{}}],"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, true)]
#[case::two_markers(r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::no_marker(r#"{"messages":[{"role":"user","content":"a"}]}"#, false)]
#[case::thinking_block(r#"{"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"},{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::null_tool_cache_control(r#"{"tools":[{"name":"t","input_schema":{},"cache_control":null}],"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::null_system_cache_control(r#"{"system":[{"type":"text","text":"s","cache_control":null}],"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::null_request_cache_control(r#"{"cache_control":null,"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::tool_choice(r#"{"tool_choice":{"type":"auto"},"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::system_role(r#"{"messages":[{"role":"system","content":"s"},{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::blank_string_message(r#"{"messages":[{"role":"user","content":" \t"},{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::empty_string_message(r#"{"messages":[{"role":"user","content":""},{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::blank_system_text(r#"{"system":[{"type":"text","text":" "}],"messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#, false)]
#[case::not_json("{", false)]
fn prediction_view_is_stricter_than_the_plan(#[case] body: &str, #[case] predicted: bool) {
    assert_eq!(parse_prompt(body).is_some(), predicted);
}

#[test]
fn prediction_reuses_the_single_breakpoint() {
    let prefix = parse_prompt(MARKED).unwrap();
    let boundary = only(MARKED);
    assert_eq!(
        prefix.fingerprint,
        "9c8b83544966d86840d97a5b2c8b7e6b4cfc8e51cb4df87fdc0d569dbaf3d998"
    );
    assert_eq!(prefix.fingerprint, boundary.fingerprint);
    assert_eq!(prefix.fingerprints, boundary.lookback_fingerprints);
    assert_eq!(prefix.ttl_seconds, 300);
    assert_eq!(prefix.prefix_body.get(), boundary.prefix_body.get());
}

#[test]
fn cache_scope_matches_litellm() {
    assert_eq!(
        cache_scope(
            "caller",
            "deployment",
            "key",
            "claude-sonnet-5",
            "2023-06-01"
        )
        .unwrap(),
        "fe4d5470d0c652f15fc02a884968aaa32a2aeed61d8cbbafabdf0d583203de98"
    );
}

#[test]
fn replays_the_cache_plan_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cache_plan_decoder");
    let outcomes: Vec<(bool, bool)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let body = str::from_utf8(&bytes).unwrap_or_default();
            (parse_cache_plan(body).is_ok(), parse_prompt(body).is_some())
        })
        .collect();
    assert!(outcomes.contains(&(true, true)));
    assert!(outcomes.contains(&(true, false)));
    assert!(outcomes.contains(&(false, false)));
}
