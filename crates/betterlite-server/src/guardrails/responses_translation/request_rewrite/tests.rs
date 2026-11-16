#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde_json::{Value, json};

pub(in crate::guardrails::responses_translation) mod fake;

use self::fake::FakeGuardrail;
use super::error::RequestRewriteError;
use super::patch::patched_request_fields;
use super::texts::{apply_guardrail_responses_to_input, apply_guardrailed_texts};
use super::write_back::written_back_request_fields;
use super::{
    ExtractedInputs, RequestContext, RequestFields, ScopedRequest, extract_guardrail_inputs,
    get_structured_messages, process_input_messages, scannable_instructions,
};

fn cache_with(entries: &[(&str, &str)]) -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    let cache = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    );
    for (key, value) in entries {
        cache.set(
            key,
            serde_json::from_str::<CacheValue>(value).unwrap(),
            Ttl::Absent,
        );
    }
    cache
}

fn context(cache: &DualLocalCache, skip_system_message: bool) -> RequestContext<'_> {
    RequestContext {
        skip_system_message,
        guardrail_name: Some("guard"),
        limits: ApiLimits::default(),
        cache,
    }
}

fn object(value: Value) -> Object {
    serde_json::from_value(value).unwrap()
}

fn strings(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(|item| (*item).to_owned()).collect())
}

fn messages(data: &Value) -> Vec<Value> {
    let cache = cache_with(&[]);
    get_structured_messages(&object(data.clone()), &context(&cache, false))
        .unwrap()
        .unwrap()
}

fn extract(request: Value, skip_system: bool) -> ExtractedInputs {
    let data = object(request);
    let input = data.get("input").cloned().unwrap();
    extract_guardrail_inputs(&data, &input, &[], skip_system).unwrap()
}

async fn process(
    request: Value,
    guardrail: &FakeGuardrail,
    skip_system: bool,
) -> (Result<(), RequestRewriteError>, Value) {
    let cache = cache_with(&[]);
    let mut data = object(request);
    let result =
        process_input_messages(&mut data, guardrail, &context(&cache, skip_system), None).await;
    (result, serde_json::to_value(&data).unwrap())
}

#[rstest]
#[case::text(json!({"instructions": "be brief"}), false, Some("be brief"))]
#[case::system_skipped(json!({"instructions": "be brief"}), true, None)]
#[case::empty(json!({"instructions": ""}), false, None)]
#[case::null(json!({"instructions": null}), false, None)]
#[case::number(json!({"instructions": 3}), false, None)]
#[case::absent(json!({}), false, None)]
fn instructions_are_scannable_only_as_non_empty_text(
    #[case] data: Value,
    #[case] skip_system: bool,
    #[case] expected: Option<&str>,
) {
    assert_eq!(scannable_instructions(&object(data), skip_system), expected);
}

#[rstest]
#[case::missing(json!({}))]
#[case::null(json!({"input": null}))]
#[case::empty_list(json!({"input": []}))]
#[case::no_content(json!({"input": [{"role": "user"}]}))]
fn structured_messages_are_none_without_messages(#[case] data: Value) {
    let cache = cache_with(&[]);
    let found = get_structured_messages(&object(data), &context(&cache, false)).unwrap();
    assert_eq!(found, None);
}

#[test]
fn structured_messages_lead_with_the_instructions() {
    let found = messages(&json!({"input": "hi", "instructions": "be brief"}));
    assert_eq!(
        found,
        [
            json!({"role": "system", "content": "be brief"}),
            json!({"role": "user", "content": "hi"})
        ]
    );
}

#[test]
fn structured_messages_reject_instructions_that_are_not_text() {
    let cache = cache_with(&[]);
    let data = object(json!({"input": "hi", "instructions": 5}));
    let error = get_structured_messages(&data, &context(&cache, false)).unwrap_err();
    assert!(matches!(
        error,
        RequestRewriteError::FieldType {
            field: "instructions"
        }
    ));
}

#[test]
fn extract_inputs_collects_texts_mappings_and_image_urls() {
    let extracted = extract(
        json!({"instructions": "sys text", "input": [
            {"role": "user", "content": "hello"},
            {"role": "system", "content": "rules"},
            "stray string",
            {"role": "User", "content": [
                {"type": "input_text", "text": "a"},
                {"type": "input_image"},
                {"type": "image_url", "image_url": {"url": "https://x.test/i.png"}, "text": "b"}
            ]},
            {"role": "assistant", "content": null},
            {"role": "assistant"}
        ]}),
        false,
    );
    assert_eq!(
        extracted.inputs.texts,
        strings(&["sys text", "hello", "rules", "a", "b"])
    );
    assert_eq!(extracted.inputs.images, strings(&["https://x.test/i.png"]));
    assert_eq!(
        extracted.task_mappings,
        [(0, None), (1, None), (3, Some(0)), (3, Some(2))]
    );
    assert_eq!(extracted.instructions.as_deref(), Some("sys text"));
}

#[test]
fn extract_inputs_skips_system_items_and_instructions_when_asked() {
    let extracted = extract(
        json!({"instructions": "sys text", "input": [
            {"role": "SYSTEM", "content": "rules"},
            {"role": "user", "content": "hello"}
        ]}),
        true,
    );
    assert_eq!(extracted.inputs.texts, strings(&["hello"]));
    assert_eq!(extracted.task_mappings, [(1, None)]);
    assert_eq!(extracted.instructions, None);
}

#[test]
fn extract_inputs_reads_a_string_input_as_one_text_without_mappings() {
    let extracted = extract(json!({"input": "hi"}), false);
    assert_eq!(extracted.inputs.texts, strings(&["hi"]));
    assert!(extracted.task_mappings.is_empty());
}

#[rstest]
#[case::empty_url(json!({"type": "image_url", "image_url": {"url": ""}}))]
#[case::other_type(json!({"type": "input_image", "image_url": {"url": "https://x.test/i"}}))]
#[case::url_not_an_object(json!({"type": "image_url", "image_url": "https://x.test/i"}))]
#[case::url_not_text(json!({"type": "image_url", "image_url": {"url": 5}}))]
fn extract_inputs_leaves_images_out_unless_the_entry_has_a_url(#[case] entry: Value) {
    let extracted = extract(
        json!({"input": [{"role": "user", "content": [entry]}]}),
        false,
    );
    assert_eq!(extracted.inputs.images, None);
}

#[test]
fn extract_inputs_deep_copies_every_flattened_tool() {
    let data = object(json!({"input": "hi"}));
    let input = data.get("input").cloned().unwrap();
    let first: Fields =
        serde_json::from_value(json!({"type": "function", "function": {"name": "f"}})).unwrap();
    let second: Fields =
        serde_json::from_value(json!({"type": "mcp", "server_label": "s"})).unwrap();
    let groups = [vec![first], vec![], vec![second]];
    let extracted = extract_guardrail_inputs(&data, &input, &groups, false).unwrap();
    assert_eq!(
        serde_json::to_value(extracted.inputs.tools).unwrap(),
        json!([{"type": "function", "function": {"name": "f"}}, {"type": "mcp", "server_label": "s"}])
    );
}

#[rstest]
#[case::empty_text(json!({"input": "hi", "model": ""}), Some(""))]
#[case::text(json!({"input": "hi", "model": "gpt"}), Some("gpt"))]
#[case::number(json!({"input": "hi", "model": 5}), None)]
#[case::absent(json!({"input": "hi"}), None)]
fn extract_inputs_sets_the_model_for_any_text(#[case] data: Value, #[case] model: Option<&str>) {
    let extracted = extract(data, false);
    assert_eq!(extracted.inputs.model.as_deref(), model);
    assert_eq!(extracted.inputs.tools, None);
    assert_eq!(extracted.inputs.images, None);
}

fn texts_outcome(request: Value, reply: &[&str]) -> (Result<(), RequestRewriteError>, Value) {
    let mut data = object(request);
    let input = data.get("input").cloned().unwrap();
    let extracted = extract_guardrail_inputs(&data, &input, &[], false).unwrap();
    let answer: GuardrailApiInputs = serde_json::from_value(json!({"texts": reply})).unwrap();
    let result = apply_guardrailed_texts(&mut data, &input, &extracted, &answer, Some("guard"));
    (result, serde_json::to_value(&data).unwrap())
}

#[test]
fn texts_write_instructions_and_a_string_input() {
    let (result, data) = texts_outcome(json!({"instructions": "i", "input": "hi"}), &["I", "HI"]);
    result.unwrap();
    assert_eq!(data, json!({"instructions": "I", "input": "HI"}));
}

#[test]
fn texts_write_string_content_and_text_entries_by_mapping() {
    let input = json!([
        {"role": "user", "content": "a"},
        {"role": "user", "content": [{"type": "image_url"}, {"type": "input_text", "text": "b"}]}
    ]);
    let (result, data) = texts_outcome(json!({"input": input}), &["A", "B"]);
    result.unwrap();
    assert_eq!(
        data["input"],
        json!([
            {"role": "user", "content": "A"},
            {"role": "user", "content": [{"type": "image_url"}, {"type": "input_text", "text": "B"}]}
        ])
    );
}

#[rstest]
#[case::too_few(json!({"instructions": "i", "input": "hi"}), &["only"])]
#[case::too_many(json!({"input": "hi"}), &["a", "b"])]
#[case::array_short(json!({"input": [{"role": "user", "content": "a"}, {"role": "user", "content": "b"}]}), &["a"])]
#[case::array_long(json!({"input": [{"role": "user", "content": "a"}]}), &["a", "b"])]
fn texts_with_a_wrong_count_fail_before_any_write(#[case] data: Value, #[case] reply: &[&str]) {
    let (result, after) = texts_outcome(data.clone(), reply);
    assert!(matches!(
        result,
        Err(RequestRewriteError::Unappliable(source)) if source.guardrail_name == "guard"
    ));
    assert_eq!(after, data);
}

#[test]
fn texts_change_nothing_for_an_empty_answer() {
    let data = json!({"instructions": "i", "input": "hi"});
    let (result, after) = texts_outcome(data.clone(), &[]);
    result.unwrap();
    assert_eq!(after, data);
}

#[test]
fn texts_change_nothing_for_an_absent_answer() {
    let mut data = object(json!({"input": "hi"}));
    let input = data.get("input").cloned().unwrap();
    let extracted = extract_guardrail_inputs(&data, &input, &[], false).unwrap();
    let answer = GuardrailApiInputs::default();
    apply_guardrailed_texts(&mut data, &input, &extracted, &answer, None).unwrap();
    assert_eq!(serde_json::to_value(&data).unwrap(), json!({"input": "hi"}));
}

#[test]
fn texts_pair_to_the_shorter_list_and_skip_what_cannot_take_a_text() {
    let mut items: Vec<GuardrailValue> = serde_json::from_value(json!([
        {"role": "user", "content": null},
        {"role": "user", "content": "keep"},
        {"role": "user", "content": ["not an object", {"type": "input_text", "text": "old"}]},
        {"role": "user", "content": [{"type": "input_text", "text": "old"}]},
        {"role": "user", "content": "unmapped"}
    ]))
    .unwrap();
    let responses = ["x", "y", "z", "w", "v"].map(str::to_owned);
    let mappings = [
        (0, None),
        (1, Some(0)),
        (2, Some(0)),
        (3, Some(5)),
        (9, None),
    ];
    apply_guardrail_responses_to_input(&mut items, &responses, &mappings);
    assert_eq!(
        serde_json::to_value(&items).unwrap(),
        json!([
            {"role": "user", "content": null},
            {"role": "user", "content": "keep"},
            {"role": "user", "content": ["not an object", {"type": "input_text", "text": "old"}]},
            {"role": "user", "content": [{"type": "input_text", "text": "old"}]},
            {"role": "user", "content": "unmapped"}
        ])
    );
}

fn patch_outcome<Edit>(data: &Value, edit: Edit) -> Option<RequestFields>
where
    Edit: FnOnce(&mut Vec<Value>),
{
    let cache = cache_with(&[]);
    let original = messages(data);
    let mut merged = original.clone();
    edit(&mut merged);
    patched_request_fields(
        data.get("input"),
        data.get("instructions"),
        &original,
        &merged,
        &context(&cache, false),
    )
    .unwrap()
}

#[test]
fn patch_changes_one_item_only() {
    let data = json!({"input": [
        {"role": "user", "content": "a", "extra": 1},
        {"role": "assistant", "content": "b"}
    ]});
    let fields = patch_outcome(&data, |merged| merged[1]["content"] = json!("B")).unwrap();
    assert_eq!(
        fields,
        RequestFields {
            input: vec![
                json!({"role": "user", "content": "a", "extra": 1}),
                json!({"role": "assistant", "content": "B"})
            ],
            instructions: None,
        }
    );
}

#[test]
fn patch_rewrites_the_instructions_row_and_leaves_the_items() {
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]});
    let fields = patch_outcome(&data, |merged| merged[0]["content"] = json!("new")).unwrap();
    assert_eq!(fields.instructions.as_deref(), Some("new"));
    assert_eq!(fields.input, [json!({"role": "user", "content": "a"})]);
}

#[test]
fn patch_removes_instructions_when_the_rewritten_row_has_no_content() {
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]});
    let fields = patch_outcome(&data, |merged| merged[0] = json!({"role": "system"})).unwrap();
    assert_eq!(fields.instructions, None);
}

#[test]
fn patch_keeps_the_old_instructions_when_only_an_item_changed() {
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]});
    let fields = patch_outcome(&data, |merged| merged[1]["content"] = json!("A")).unwrap();
    assert_eq!(fields.instructions.as_deref(), Some("sys"));
    assert_eq!(fields.input, [json!({"role": "user", "content": "A"})]);
}

#[test]
fn patch_counts_the_offset_for_non_empty_instructions_only() {
    let data = json!({"instructions": "", "input": [{"role": "user", "content": "a"}]});
    let fields = patch_outcome(&data, |merged| merged[0]["content"] = json!("A")).unwrap();
    assert_eq!(fields.instructions.as_deref(), Some(""));
    assert_eq!(fields.input, [json!({"role": "user", "content": "A"})]);
}

#[rstest]
#[case::instructions_not_text(
    json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]}),
    0,
    json!({"role": "system", "content": [{"type": "text", "text": "x"}]})
)]
#[case::item_cannot_be_patched(
    json!({"input": [{"type": "mystery", "role": "user", "content": "a"}]}),
    0,
    json!({"role": "user", "content": "A"})
)]
fn patch_gives_none_when_the_rewrite_cannot_be_placed(
    #[case] data: Value,
    #[case] index: usize,
    #[case] row: Value,
) {
    assert_eq!(patch_outcome(&data, |merged| merged[index] = row), None);
}

#[test]
fn patch_gives_none_for_input_that_is_not_a_list() {
    let data = json!({"input": "hi"});
    assert_eq!(
        patch_outcome(&data, |merged| merged[0]["content"] = json!("HI")),
        None
    );
}

#[test]
fn patch_gives_none_for_a_length_mismatch() {
    let cache = cache_with(&[]);
    let data = json!({"input": [{"role": "user", "content": "a"}]});
    let original = messages(&data);
    let merged = [original[0].clone(), original[0].clone()];
    let found = patched_request_fields(
        data.get("input"),
        None,
        &original,
        &merged,
        &context(&cache, false),
    );
    assert_eq!(found.unwrap(), None);
}

#[test]
fn patch_gives_none_when_the_raw_items_do_not_produce_the_messages() {
    let cache = cache_with(&[]);
    let raw = json!([{"role": "user", "content": "a"}]);
    let original = [json!({"role": "assistant", "content": "a"})];
    let merged = [json!({"role": "assistant", "content": "A"})];
    let found = patched_request_fields(
        Some(&raw),
        None,
        &original,
        &merged,
        &context(&cache, false),
    );
    assert_eq!(found.unwrap(), None);
}

#[test]
fn patch_gives_none_for_a_tainted_message() {
    let data = json!({"input": [
        {"role": "assistant", "content": "hello"},
        {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}
    ]});
    assert_eq!(
        patch_outcome(&data, |merged| merged[0]["content"] = json!("changed")),
        None
    );
}

#[test]
fn patch_gives_none_for_two_changes_in_one_item() {
    let call = r#"{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#;
    let cache = cache_with(&[("c1", call)]);
    let data = json!({"input": [{"type": "function_call_output", "call_id": "c1", "output": "r"}]});
    let original = [
        json!({"role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}]}),
        json!({"role": "tool", "content": "r", "tool_call_id": "c1"}),
    ];
    let mut merged = original.clone();
    merged[0]["content"] = json!("x");
    merged[1]["content"] = json!("R");
    let found = patched_request_fields(
        data.get("input"),
        None,
        &original,
        &merged,
        &context(&cache, false),
    );
    assert_eq!(found.unwrap(), None);
}

fn scoped(rows: &[Value], indices: &[usize]) -> ScopedRequest {
    let sent: Vec<GuardrailMessage> = indices
        .iter()
        .map(|index| serde_json::from_value(rows[*index].clone()).unwrap())
        .collect();
    ScopedRequest {
        structured_messages: rows.to_vec(),
        scoped_indices: indices.to_vec(),
        scoped_messages: Some(sent),
    }
}

#[rstest]
#[case::no_list(json!({}))]
#[case::the_list_that_was_sent(json!({"structured_messages": [{"role": "user", "content": "a"}]}))]
fn write_back_ignores_an_answer_without_new_messages(#[case] reply: Value) {
    let cache = cache_with(&[]);
    let data = object(json!({"input": [{"role": "user", "content": "a"}]}));
    let rows = [json!({"role": "user", "content": "a"})];
    let answer: GuardrailApiInputs = serde_json::from_value(reply).unwrap();
    let found = written_back_request_fields(
        &data,
        &scoped(&rows, &[0]),
        &answer,
        false,
        &context(&cache, false),
    );
    assert_eq!(found.unwrap(), None);
}

#[tokio::test]
async fn write_back_converts_a_string_input_into_an_array_and_sets_instructions() {
    let reply = json!({"structured_messages": [
        {"role": "system", "content": "SYS"}, {"role": "user", "content": "HI"}
    ]});
    let guardrail = FakeGuardrail::replying(reply);
    let (result, data) = process(
        json!({"input": "hi", "instructions": "sys"}),
        &guardrail,
        false,
    )
    .await;
    result.unwrap();
    assert_eq!(
        data,
        json!({"instructions": "SYS", "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "HI"}]}
        ]})
    );
}

#[tokio::test]
async fn write_back_removes_instructions_the_conversion_does_not_produce() {
    let reply = json!({"structured_messages": [{"role": "user", "content": "HI"}]});
    let guardrail = FakeGuardrail::replying(reply);
    let (result, data) = process(
        json!({"input": "hi", "instructions": "sys"}),
        &guardrail,
        false,
    )
    .await;
    result.unwrap();
    assert_eq!(data.get("instructions"), None);
}

#[tokio::test]
async fn write_back_patches_an_array_input_item_by_item() {
    let reply = json!({"structured_messages": [
        {"role": "user", "content": "a"}, {"role": "assistant", "content": "B"}
    ]});
    let input = json!([{"role": "user", "content": "a", "extra": 1}, {"role": "assistant", "content": "b"}]);
    let (result, data) = process(
        json!({"input": input}),
        &FakeGuardrail::replying(reply),
        false,
    )
    .await;
    result.unwrap();
    assert_eq!(
        data["input"],
        json!([{"role": "user", "content": "a", "extra": 1}, {"role": "assistant", "content": "B"}])
    );
}

#[tokio::test]
async fn write_back_merges_a_scoped_list_into_the_full_list() {
    let reply = json!({"structured_messages": [{"role": "user", "content": "A"}]});
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]});
    let (result, after) = process(data, &FakeGuardrail::replying(reply), true).await;
    result.unwrap();
    assert_eq!(
        after,
        json!({"instructions": "sys", "input": [{"role": "user", "content": "A"}]})
    );
}

#[tokio::test]
async fn write_back_installs_a_list_the_guardrail_says_covers_the_request() {
    let reply = json!({"structured_messages": [
        {"role": "system", "content": "S2"}, {"role": "user", "content": "A"}
    ]});
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]});
    let guardrail = FakeGuardrail::replying(reply).covering_full_requests();
    let (result, after) = process(data, &guardrail, true).await;
    result.unwrap();
    assert_eq!(
        after,
        json!({"instructions": "S2", "input": [{"role": "user", "content": "A"}]})
    );
}

#[tokio::test]
async fn write_back_merges_a_list_of_the_full_length_when_the_guardrail_does_not_cover_it() {
    let reply = json!({"structured_messages": [
        {"role": "system", "content": "S2"}, {"role": "user", "content": "A"}
    ]});
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "a"}]});
    let (result, after) = process(data, &FakeGuardrail::replying(reply), true).await;
    result.unwrap();
    assert_eq!(
        after,
        json!({"instructions": "sys S2", "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "A"}]}
        ]})
    );
}

#[tokio::test]
async fn write_back_falls_back_to_conversion_for_an_item_that_cannot_be_patched() {
    let reply = json!({"structured_messages": [{"role": "user", "content": "A"}]});
    let input = json!([{"type": "mystery", "role": "user", "content": "a"}]);
    let (result, data) = process(
        json!({"input": input}),
        &FakeGuardrail::replying(reply),
        false,
    )
    .await;
    result.unwrap();
    assert_eq!(
        data["input"],
        json!([{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "A"}]}])
    );
}

#[rstest]
#[case::only_a_system_item(json!({"input": [{"role": "system", "content": "s"}]}))]
#[case::empty_list(json!({"input": []}))]
#[case::input_is_a_number(json!({"input": 5}))]
#[case::input_is_absent(json!({"model": "m"}))]
#[tokio::test]
async fn process_input_makes_no_call_without_texts(#[case] data: Value) {
    let guardrail = FakeGuardrail::echoing();
    let (result, after) = process(data.clone(), &guardrail, true).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, data);
}

#[tokio::test]
async fn process_input_sends_the_texts_the_scoped_messages_and_the_request_data() {
    let data =
        json!({"model": "m", "instructions": "sys", "input": [{"role": "user", "content": "hi"}]});
    let guardrail = FakeGuardrail::echoing();
    let (result, after) = process(data.clone(), &guardrail, true).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(calls[0].inputs.texts, strings(&["hi"]));
    assert_eq!(calls[0].inputs.model.as_deref(), Some("m"));
    let sent = serde_json::to_value(&calls[0].inputs.structured_messages).unwrap();
    assert_eq!(sent, json!([{"role": "user", "content": "hi"}]));
    assert_eq!(serde_json::to_value(&calls[0].request_data).unwrap(), data);
    assert_eq!(after, data);
}

#[tokio::test]
async fn process_input_scans_the_instructions_when_system_messages_are_not_skipped() {
    let data = json!({"instructions": "sys", "input": [{"role": "user", "content": "hi"}]});
    let guardrail = FakeGuardrail::echoing();
    let (result, _after) = process(data, &guardrail, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls[0].inputs.texts, strings(&["sys", "hi"]));
    assert_eq!(
        calls[0].inputs.structured_messages.as_ref().map(Vec::len),
        Some(2)
    );
}

#[tokio::test]
async fn process_input_merges_returned_tools_and_keeps_them_when_the_write_back_falls_back() {
    let tool = json!({"type": "function", "name": "f", "description": "d", "parameters": {"type": "object"}});
    let reply = json!({"tools": [], "structured_messages": [{"role": "user", "content": "HI"}]});
    let guardrail = FakeGuardrail::replying(reply);
    let (result, data) = process(json!({"input": "hi", "tools": [tool]}), &guardrail, false).await;
    result.unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.tools.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(data["tools"], json!([]));
    assert_eq!(
        data["input"],
        json!([{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "HI"}]}])
    );
}

#[tokio::test]
async fn process_input_writes_structured_messages_before_texts() {
    let reply = json!({"texts": ["T"], "structured_messages": [{"role": "user", "content": "S"}]});
    let data = json!({"input": [{"role": "user", "content": "a"}]});
    let (result, after) = process(data, &FakeGuardrail::replying(reply), false).await;
    result.unwrap();
    assert_eq!(after["input"], json!([{"role": "user", "content": "S"}]));
}

#[tokio::test]
async fn process_input_writes_texts_when_no_messages_come_back() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HI"]}));
    let (result, data) = process(json!({"input": "hi"}), &guardrail, false).await;
    result.unwrap();
    assert_eq!(data, json!({"input": "HI"}));
}

#[tokio::test]
async fn process_input_rejects_a_text_rewrite_it_cannot_place() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["only one"]}));
    let data = json!({"instructions": "sys", "input": "hi"});
    let (result, after) = process(data.clone(), &guardrail, false).await;
    assert!(matches!(result, Err(RequestRewriteError::Unappliable(_))));
    assert_eq!(after, data);
}

#[tokio::test]
async fn process_input_passes_a_guardrail_failure_through() {
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let data = json!({"input": "hi"});
    let (result, after) = process(data.clone(), &guardrail, false).await;
    assert!(matches!(
        result,
        Err(RequestRewriteError::Guardrail { source }) if *source == failure
    ));
    assert_eq!(after, data);
}
