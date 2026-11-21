#![cfg(test)]

mod support;

use std::borrow::Cow;
use std::iter::repeat_n;
use std::sync::{Arc, Mutex};

use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use rstest::rstest;

use self::support::{
    Client, MaskCall, Masker, QuotaCall, Verdict, WRAPPED_CONTENT, WRAPPED_ITEM_ID,
    WRAPPED_RESPONSE_ID, echo, json, key, lock, masker, object, parsed, quota, recording_unmask,
    request_data,
};
use super::events::{wrap_item_encrypted_content, ws_event_error};
use super::quota::{
    FrameQuotaEstimate, enforce_frame_project_quota, enforce_or_reject_frame, frame_quota_estimate,
};
use super::request::{
    enforce_authorized_model, mask_response_create, restore_wrapped_ids as restore_ids,
    with_request_defaults,
};
use super::response::{mask_response_completed, unmask_response_event};
use super::{
    CHARS_PER_TOKEN_ESTIMATE, DELTA_EVENT_TYPES, FAILURE_EVENT_TYPES, FramePolicy,
    FramePolicyError, InputGuardrail, LOGGED_EVENT_TYPES, MASKABLE_TEXT_BLOCK_TYPES,
    NO_MAX_OUTPUT_TOKENS_FLOOR, OUTPUT_DONE_EVENT_TYPES, OUTPUT_ITEM_EVENT_TYPES,
};

const CREATE: &str = r#"{"type":"response.create","input":"abcd","max_output_tokens":7}"#;
const TOKENS: &str = r#"{"metadata":{"pii_tokens":{"<EMAIL_ADDRESS_1>":"a@example.com"}}}"#;

#[rstest]
#[case::flat(
    r#"{"type":"response.create","input":"abcd","instructions":"efgh","max_output_tokens":7}"#,
    2,
    7
)]
#[case::nested(
    r#"{"type":"response.create","response":{"input":"abcd","instructions":"efgh","max_output_tokens":7}}"#,
    2,
    7
)]
#[case::floor_without_cap(r#"{"type":"response.create","input":"abcde"}"#, 1, 1024)]
#[case::item_content_and_text(
    r#"{"type":"response.create","input":[{"content":"ab"},{"text":"cd"}]}"#,
    1,
    1024
)]
#[case::no_text(
    r#"{"type":"response.create","input":"","instructions":"","max_output_tokens":null}"#,
    0,
    1024
)]
#[case::short_text_rounds_up(r#"{"type":"response.create","input":"abc"}"#, 1, 1024)]
#[case::string_cap(
    r#"{"type":"response.create","input":"abcd","max_output_tokens":"7"}"#,
    1,
    1024
)]
#[case::float_cap(
    r#"{"type":"response.create","input":"abcd","max_output_tokens":7.0}"#,
    1,
    1024
)]
#[case::true_cap(
    r#"{"type":"response.create","input":"abcd","max_output_tokens":true}"#,
    1,
    1
)]
#[case::false_cap(
    r#"{"type":"response.create","input":"abcd","max_output_tokens":false}"#,
    1,
    0
)]
#[case::empty_response_falls_back(
    r#"{"type":"response.create","response":{},"input":[{"content":"abcd","id":"encitem_1"}],"max_output_tokens":7}"#,
    1,
    7
)]
#[case::nested_wins(
    r#"{"type":"response.create","response":{"input":"abcdefgh"},"input":"ab","max_output_tokens":7}"#,
    2,
    1024
)]
#[case::only_strings_and_object_text(
    r#"{"type":"response.create","input":["ab",["cdefgh"],5,{"content":[{"text":"wxyz"}]}],"instructions":{"text":"ignored"}}"#,
    1,
    1024
)]
#[case::code_points(r#"{"type":"response.create","input":"éééééééé"}"#, 2, 1024)]
fn quota_estimation_shapes(
    #[case] frame: &str,
    #[case] input_tokens: i64,
    #[case] output_tokens: i64,
) {
    assert_eq!(NO_MAX_OUTPUT_TOKENS_FLOOR, 1024);
    assert_eq!(CHARS_PER_TOKEN_ESTIMATE, 4);
    assert_eq!(
        frame_quota_estimate(&object(frame)),
        FrameQuotaEstimate {
            input_tokens,
            output_tokens
        }
    );
}

#[rstest]
#[case::not_json("not-json", Verdict::Accept, 0)]
#[case::array("[1,2,3]", Verdict::Accept, 0)]
#[case::completed_event(r#"{"type":"response.completed"}"#, Verdict::Accept, 0)]
#[case::ping(r#"{"type":"ping"}"#, Verdict::Accept, 0)]
#[case::create(CREATE, Verdict::Accept, 2)]
#[case::first_raises(CREATE, Verdict::Broken, 1)]
#[tokio::test]
async fn quota_frame_selection(
    #[case] frame: &str,
    #[case] first: Verdict,
    #[case] charged: usize,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callbacks = [
        quota("one", first, &calls),
        quota("two", Verdict::Accept, &calls),
    ];
    let user = key();
    let outcome = enforce_frame_project_quota(&callbacks, Some(&user), Some("gpt-4o"), frame).await;
    assert_eq!(
        matches!(outcome, Err(ErrorClass::InternalServer(_))),
        first == Verdict::Broken
    );
    let expected: Vec<QuotaCall> = ["one", "two"]
        .into_iter()
        .take(charged)
        .map(|name| QuotaCall {
            name,
            key: Some(serde_json::to_string(&user).unwrap()),
            model: Some("gpt-4o".to_owned()),
            input_tokens: 1,
            output_tokens: 7,
        })
        .collect();
    assert_eq!(*lock(&calls), expected);
    assert_eq!(
        enforce_frame_project_quota(&[], Some(&user), None, CREATE).await,
        Ok(())
    );
}

#[test]
fn ws_event_error_and_event_sets() {
    let code = json(r#"{"code":"E"}"#);
    let event_error = object(r#"{"type":"error","error":{"code":"E"}}"#);
    assert_eq!(ws_event_error(&event_error), Some(&code));
    assert_eq!(ws_event_error(&object(r#"{"type":"error"}"#)), None);
    let error_ignores_response = object(r#"{"type":"error","response":{"error":{"code":"E"}}}"#);
    assert_eq!(ws_event_error(&error_ignores_response), None);
    let completed = object(r#"{"type":"response.completed","response":{"error":{"code":"E"}}}"#);
    assert_eq!(ws_event_error(&completed), Some(&code));
    assert_eq!(
        ws_event_error(&object(r#"{"type":"response.completed"}"#)),
        None
    );
    let listed = object(r#"{"type":"response.completed","response":[]}"#);
    assert_eq!(ws_event_error(&listed), None);
    let other = object(r#"{"type":"other","error":{"code":"E"}}"#);
    assert_eq!(ws_event_error(&other), None);
    assert_eq!(
        LOGGED_EVENT_TYPES,
        [
            "response.created",
            "response.completed",
            "response.failed",
            "response.incomplete",
            "error"
        ]
    );
    assert_eq!(FAILURE_EVENT_TYPES, ["error", "response.failed"]);
    assert_eq!(
        OUTPUT_ITEM_EVENT_TYPES,
        ["response.output_item.added", "response.output_item.done"]
    );
    assert_eq!(
        MASKABLE_TEXT_BLOCK_TYPES,
        ["input_text", "output_text", "text"]
    );
    assert_eq!(
        DELTA_EVENT_TYPES,
        [
            "response.output_text.delta",
            "response.reasoning_summary_text.delta",
            "response.refusal.delta",
            "response.function_call_arguments.delta",
        ]
    );
    assert_eq!(
        OUTPUT_DONE_EVENT_TYPES,
        [
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.function_call_arguments.done",
            "response.reasoning_summary_text.done",
            "response.reasoning_summary_part.done",
        ]
    );
}

#[rstest]
#[case::flat_ids_restored(
    r#"{"type":"response.create","input":[{"id":"ITEM","encrypted_content":"CONTENT","role":"user"}],"previous_response_id":"RESPONSE","response":{"input":[],"previous_response_id":"original"}}"#,
    Some(
        r#"{"type":"response.create","input":[{"id":"item-9","encrypted_content":"cipher","role":"user"}],"previous_response_id":"resp_orig","response":{"input":[],"previous_response_id":"original"}}"#
    )
)]
#[case::already_original(
    r#"{"type":"response.create","input":[{"id":"encitem_1","encrypted_content":"litellm_enc:x"}],"previous_response_id":"wrapped","response":{"input":[],"previous_response_id":"original"}}"#,
    None
)]
#[case::nested_only(
    r#"{"type":"response.create","response":{"input":[{"id":"ITEM"}],"previous_response_id":"RESPONSE"}}"#,
    Some(
        r#"{"type":"response.create","response":{"input":[{"id":"item-9"}],"previous_response_id":"resp_orig"}}"#
    )
)]
#[case::nested_previous_only(
    r#"{"type":"response.create","input":"text","response":{"previous_response_id":"RESPONSE"}}"#,
    Some(
        r#"{"type":"response.create","input":"text","response":{"previous_response_id":"resp_orig"}}"#
    )
)]
#[case::empty_nested_stays_nested(
    r#"{"type":"response.create","response":{},"input":[{"content":"abcd","id":"ITEM"}],"max_output_tokens":7}"#,
    Some(
        r#"{"type":"response.create","response":{},"input":[{"content":"abcd","id":"item-9"}],"max_output_tokens":7}"#
    )
)]
#[case::nothing_to_restore(
    r#"{"type":"response.create","input":"text","previous_response_id":5}"#,
    None
)]
fn restore_wrapped_ids(#[case] frame: &str, #[case] restored: Option<&str>) {
    let wrapped = frame
        .replace("ITEM", WRAPPED_ITEM_ID)
        .replace("CONTENT", WRAPPED_CONTENT)
        .replace("RESPONSE", WRAPPED_RESPONSE_ID);
    let original = object(&wrapped);
    assert_eq!(restore_ids(&original), restored.map(object));
    assert_eq!(original, object(&wrapped));
}

#[rstest]
#[case::added(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"deploy-7"}}"#,
    r#"{"type":"response.output_item.added","item":{"id":"rs_1","encrypted_content":"cipher"}}"#,
    Some("litellm_enc:bW9kZWxfaWQ6ZGVwbG95LTc=;cipher")
)]
#[case::done(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"deploy-7"}}"#,
    r#"{"type":"response.output_item.done","item":{"id":"rs_1","encrypted_content":"cipher"}}"#,
    Some("litellm_enc:bW9kZWxfaWQ6ZGVwbG95LTc=;cipher")
)]
#[case::any_event(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"deploy-7"}}"#,
    r#"{"type":"anything","item":{"id":"rs_1","encrypted_content":"cipher"}}"#,
    Some("litellm_enc:bW9kZWxfaWQ6ZGVwbG95LTc=;cipher")
)]
#[case::empty_model_id(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":""}}"#,
    r#"{"type":"response.output_item.added","item":{"id":"rs_1","encrypted_content":"cipher"}}"#,
    Some("litellm_enc:bW9kZWxfaWQ6;cipher")
)]
#[case::missing_model_info(
    r#"{"encrypted_content_affinity_enabled":true}"#,
    r#"{"item":{"encrypted_content":"cipher"}}"#,
    None
)]
#[case::null_model_info(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":null}"#,
    r#"{"item":{"encrypted_content":"cipher"}}"#,
    None
)]
#[case::numeric_model_id(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":7}}"#,
    r#"{"item":{"encrypted_content":"cipher"}}"#,
    None
)]
#[case::missing_item(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"m"}}"#,
    r#"{"type":"response.output_item.added"}"#,
    None
)]
#[case::text_item(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"m"}}"#,
    r#"{"item":"cipher"}"#,
    None
)]
#[case::missing_content(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"m"}}"#,
    r#"{"item":{"id":"rs_1"}}"#,
    None
)]
#[case::empty_content(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"m"}}"#,
    r#"{"item":{"encrypted_content":""}}"#,
    None
)]
#[case::numeric_content(
    r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"m"}}"#,
    r#"{"item":{"encrypted_content":5}}"#,
    None
)]
#[case::affinity_false(
    r#"{"encrypted_content_affinity_enabled":false,"model_info":{"id":"m"}}"#,
    r#"{"item":{"encrypted_content":"cipher"}}"#,
    None
)]
#[case::affinity_null(
    r#"{"encrypted_content_affinity_enabled":null,"model_info":{"id":"m"}}"#,
    r#"{"item":{"encrypted_content":"cipher"}}"#,
    None
)]
#[case::affinity_missing(
    r#"{"model_info":{"id":"m"}}"#,
    r#"{"item":{"encrypted_content":"cipher"}}"#,
    None
)]
fn affinity_output_item_wrap(
    #[case] metadata: &str,
    #[case] event: &str,
    #[case] wrapped: Option<&str>,
) {
    let expected = wrapped.map(|cipher| {
        let mut changed = object(event);
        changed.insert(
            "item".to_owned(),
            json(&format!(
                r#"{{"id":"rs_1","encrypted_content":"{cipher}"}}"#
            )),
        );
        changed
    });
    assert_eq!(
        wrap_item_encrypted_content(&object(event), &object(metadata)),
        expected
    );
}

#[rstest]
#[case::flat(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","model":"client"}"#,
    r#"{"type":"response.create","model":"gpt-4o"}"#,
    true
)]
#[case::nested(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","response":{"model":"client"}}"#,
    r#"{"type":"response.create","response":{"model":"gpt-4o"}}"#,
    true
)]
#[case::both(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","model":"client","response":{"model":"client"}}"#,
    r#"{"type":"response.create","model":"gpt-4o","response":{"model":"gpt-4o"}}"#,
    true
)]
#[case::top_level_only(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","model":"client","response":{"model":"gpt-4o"}}"#,
    r#"{"type":"response.create","model":"gpt-4o","response":{"model":"gpt-4o"}}"#,
    true
)]
#[case::inserted(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create"}"#,
    r#"{"type":"response.create","model":"gpt-4o"}"#,
    true
)]
#[case::empty_nested(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","response":{}}"#,
    r#"{"type":"response.create","response":{"model":"gpt-4o"}}"#,
    true
)]
#[case::text_response(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","response":"x"}"#,
    r#"{"type":"response.create","response":"x","model":"gpt-4o"}"#,
    true
)]
#[case::already_flat(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","model":"gpt-4o"}"#,
    r#"{"type":"response.create","model":"gpt-4o"}"#,
    false
)]
#[case::already_nested(
    Some("gpt-4o"),
    None,
    r#"{"type":"response.create","model":"gpt-4o","response":{"model":"gpt-4o"}}"#,
    r#"{"type":"response.create","model":"gpt-4o","response":{"model":"gpt-4o"}}"#,
    false
)]
#[case::no_model(
    None,
    None,
    r#"{"type":"response.create","model":"client"}"#,
    r#"{"type":"response.create","model":"client"}"#,
    false
)]
#[case::empty_model(
    Some(""),
    None,
    r#"{"type":"response.create","model":"client"}"#,
    r#"{"type":"response.create","model":"client"}"#,
    false
)]
#[case::precedence(None, Some(r#"{"fill_missing":{"temperature":0.2,"top_p":1},"overrides":{"temperature":0.9,"type":"x"}}"#), r#"{"type":"response.create","temperature":0.5}"#, r#"{"type":"response.create","temperature":0.9,"top_p":1}"#, false)]
#[case::nested_defaults(
    None,
    Some(r#"{"fill_missing":{"top_p":1},"overrides":{"temperature":0.9}}"#),
    r#"{"type":"response.create","response":{"temperature":0.5}}"#,
    r#"{"type":"response.create","response":{"temperature":0.9,"top_p":1}}"#,
    false
)]
#[case::defaults_then_model(
    Some("gpt-4o"),
    Some(r#"{"fill_missing":{},"overrides":{"model":"override"}}"#),
    r#"{"type":"response.create","response":{}}"#,
    r#"{"type":"response.create","response":{"model":"gpt-4o"}}"#,
    true
)]
fn model_authorization_and_defaults(
    #[case] authorized_model: Option<&str>,
    #[case] defaults: Option<&str>,
    #[case] frame: &str,
    #[case] expected: &str,
    #[case] modified: bool,
) {
    let request_defaults: Option<WsRequestDefaults<Json>> =
        defaults.map(|text| serde_json::from_str(text).unwrap());
    let mut merged = with_request_defaults(request_defaults.as_ref(), &object(frame));
    assert_eq!(
        enforce_authorized_model(authorized_model, &mut merged),
        modified
    );
    assert_eq!(merged, object(expected));
    assert_eq!(with_request_defaults(None, &object(frame)), object(frame));
}

#[rstest]
#[case::not_json("not-json", None, None, None)]
#[case::ping(r#"{"type": "ping"}"#, Some("gpt-4o"), None, None)]
#[case::already_authorized(
    r#"{"type": "response.create", "model": "gpt-4o", "input": "hi"}"#,
    Some("gpt-4o"),
    None,
    None
)]
#[case::model_only(
    r#"{"type": "response.create", "model": "client"}"#,
    Some("gpt-4o"),
    None,
    Some(r#"{"type":"response.create","model":"gpt-4o"}"#)
)]
#[case::defaults_only(
    r#"{"type": "response.create"}"#,
    None,
    Some(r#"{"fill_missing":{"temperature":0.2},"overrides":{}}"#),
    Some(r#"{"type":"response.create","temperature":0.2}"#)
)]
#[case::ids_only(
    r#"{"type": "response.create", "previous_response_id": "RESPONSE"}"#,
    None,
    None,
    Some(r#"{"type":"response.create","previous_response_id":"resp_orig"}"#)
)]
#[case::all_changes(
    r#"{"type": "response.create", "model": "client", "input": [{"id": "ITEM"}]}"#,
    Some("gpt-4o"),
    Some(r#"{"fill_missing":{"temperature":0.2,"previous_response_id":"RESPONSE"},"overrides":{"model":"override"}}"#),
    Some(r#"{"type":"response.create","model":"gpt-4o","temperature":0.2,"input":[{"id":"item-9"}],"previous_response_id":"resp_orig"}"#)
)]
#[tokio::test]
async fn request_masking_without_callbacks(
    #[case] message: &str,
    #[case] authorized_model: Option<&str>,
    #[case] defaults: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let frame = message
        .replace("ITEM", WRAPPED_ITEM_ID)
        .replace("RESPONSE", WRAPPED_RESPONSE_ID);
    let defaults_text = defaults.map(|text| text.replace("RESPONSE", WRAPPED_RESPONSE_ID));
    let request_defaults: Option<WsRequestDefaults<Json>> =
        defaults_text.map(|text| serde_json::from_str(&text).unwrap());
    let policy = FramePolicy::<Masker> {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &[],
        authorized_model,
        request_defaults: request_defaults.as_ref(),
    };
    let mut data = request_data("{}");
    let masked = mask_response_create(&policy, &mut data, &frame)
        .await
        .unwrap();
    assert_eq!(
        matches!(masked, Cow::Borrowed(text) if text == frame),
        expected.is_none()
    );
    assert_eq!(parsed(&masked), parsed(expected.unwrap_or(&frame)));
    assert!(data.is_empty());
}

#[rstest]
#[case::every_text_field(
    r#"{"type":"response.create","instructions":"i a@example.com","input":[{"content":"c a@example.com"},{"output":"o"},{"content":[{"type":"input_text","text":"t1"},{"type":"output_text","text":"t2"},{"type":"text","text":"t3"},{"type":"input_image","text":"img"},{"type":"other","text":"u"},{"type":"input_text","text":5},{"type":"input_text"},"block"]},"skip",{"content":5}],"response":{"input":"n a@example.com","instructions":"ni"}}"#,
    "{}",
    "{}",
    &["c a@example.com", "o", "t1", "t2", "t3", "i a@example.com", "n a@example.com", "ni"],
    Some(r#"{"type":"response.create","instructions":"i <EMAIL>","input":[{"content":"c <EMAIL>"},{"output":"o"},{"content":[{"type":"input_text","text":"t1"},{"type":"output_text","text":"t2"},{"type":"text","text":"t3"},{"type":"input_image","text":"img"},{"type":"other","text":"u"},{"type":"input_text","text":5},{"type":"input_text"},"block"]},"skip",{"content":5}],"response":{"input":"n <EMAIL>","instructions":"ni"}}"#)
)]
#[case::equal_text_still_serializes(r#"{"type": "response.create", "input": "plain"}"#, r#"{"metadata":{"user":"x"}}"#, r#"{"user":"x"}"#, &["plain"], Some(r#"{"type":"response.create","input":"plain"}"#))]
#[case::nothing_to_mask(r#"{"type": "response.create", "input": null, "instructions": ["x"]}"#, "{}", "{}", &[], None)]
#[case::nested_only(r#"{"type": "response.create", "response": {"input": "n"}}"#, "{}", "{}", &["n"], Some(r#"{"type":"response.create","response":{"input":"n"}}"#))]
#[case::items_only(r#"{"type": "response.create", "input": [{"content": "c"}]}"#, "{}", "{}", &["c"], Some(r#"{"type":"response.create","input":[{"content":"c"}]}"#))]
#[case::blocks_only(r#"{"type": "response.create", "input": [{"output": [{"type": "text", "text": "t"}]}]}"#, "{}", "{}", &["t"], Some(r#"{"type":"response.create","input":[{"output":[{"type":"text","text":"t"}]}]}"#))]
#[case::instructions_only(r#"{"type": "response.create", "instructions": "i"}"#, "{}", "{}", &["i"], Some(r#"{"type":"response.create","instructions":"i"}"#))]
#[tokio::test]
async fn request_guardrail_masking(
    #[case] message: &str,
    #[case] stored: &str,
    #[case] metadata: &str,
    #[case] first_texts: &[&str],
    #[case] expected: Option<&str>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let guardrails = [
        InputGuardrail {
            callback: masker(0, vec![("a@example.com", "<EMAIL_ADDRESS_1>")], &calls),
            unmask_pii_text: &echo,
        },
        InputGuardrail {
            callback: masker(1, vec![("<EMAIL_ADDRESS_1>", "<EMAIL>")], &calls),
            unmask_pii_text: &echo,
        },
    ];
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &guardrails,
        output_guardrail_callbacks: &[],
        authorized_model: None,
        request_defaults: None,
    };
    let mut data = request_data(stored);
    let masked = mask_response_create(&policy, &mut data, message)
        .await
        .unwrap();
    assert_eq!(
        matches!(masked, Cow::Borrowed(text) if text == message),
        expected.is_none()
    );
    assert_eq!(parsed(&masked), parsed(expected.unwrap_or(message)));
    assert_eq!(data.get("metadata").map(|raw| raw.get()), Some(metadata));
    let chained = first_texts
        .iter()
        .map(|text| (1_usize, text.replace("a@example.com", "<EMAIL_ADDRESS_1>")));
    let expected_calls: Vec<MaskCall> = first_texts
        .iter()
        .map(|text| (0_usize, (*text).to_owned()))
        .chain(chained)
        .map(|(guard, text)| MaskCall {
            guard,
            text,
            output_parse_pii: true,
            metadata: Some(metadata.to_owned()),
        })
        .collect();
    assert_eq!(*lock(&calls), expected_calls);
}

#[rstest]
#[case::not_json("not-json", TOKENS, None, &[])]
#[case::no_metadata(r#"{"type":"response.output_text.delta","delta":"<EMAIL_ADDRESS_1>"}"#, "{}", None, &[])]
#[case::text_metadata(r#"{"type":"response.output_text.delta","delta":"<EMAIL_ADDRESS_1>"}"#, r#"{"metadata":"x"}"#, None, &[])]
#[case::numeric_tokens(r#"{"type":"response.output_text.delta","delta":"<EMAIL_ADDRESS_1>"}"#, r#"{"metadata":{"pii_tokens":{"<EMAIL_ADDRESS_1>":5}}}"#, None, &[])]
#[case::empty_tokens(r#"{"type":"response.output_text.delta","delta":"<EMAIL_ADDRESS_1>"}"#, r#"{"metadata":{"pii_tokens":{}}}"#, None, &[])]
#[case::completed(
    r#"{"type":"response.completed","response":{"output":[{"content":[{"text":"hi <EMAIL_ADDRESS_1>"},{"text":"plain"},{"text":7},"x"]},"y",{"content":"z"}]}}"#,
    TOKENS,
    Some(r#"{"type":"response.completed","response":{"output":[{"content":[{"text":"hi a@example.com"},{"text":"plain"},{"text":7},"x"]},"y",{"content":"z"}]}}"#),
    &["hi <EMAIL_ADDRESS_1>", "plain"]
)]
#[case::completed_without_tokens(r#"{"type": "response.completed", "response": {"output": [{"content": [{"text": "plain"}]}]}}"#, TOKENS, None, &["plain"])]
#[case::completed_text_response(r#"{"type":"response.completed","response":"x"}"#, TOKENS, None, &[])]
#[case::output_text_delta(r#"{"type":"response.output_text.delta","delta":"to <EMAIL_ADDRESS_1>"}"#, TOKENS, Some(r#"{"type":"response.output_text.delta","delta":"to a@example.com"}"#), &["to <EMAIL_ADDRESS_1>"])]
#[case::summary_delta(r#"{"type":"response.reasoning_summary_text.delta","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, Some(r#"{"type":"response.reasoning_summary_text.delta","delta":"a@example.com"}"#), &["<EMAIL_ADDRESS_1>"])]
#[case::refusal_delta(r#"{"type":"response.refusal.delta","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, Some(r#"{"type":"response.refusal.delta","delta":"a@example.com"}"#), &["<EMAIL_ADDRESS_1>"])]
#[case::arguments_delta(r#"{"type":"response.function_call_arguments.delta","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, Some(r#"{"type":"response.function_call_arguments.delta","delta":"a@example.com"}"#), &["<EMAIL_ADDRESS_1>"])]
#[case::equal_delta(r#"{"type": "response.output_text.delta", "delta": "plain"}"#, TOKENS, None, &["plain"])]
#[case::numeric_delta(r#"{"type":"response.output_text.delta","delta":5}"#, TOKENS, None, &[])]
#[case::output_text_done(r#"{"type":"response.output_text.done","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
#[case::content_part_done(r#"{"type":"response.content_part.done","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
#[case::output_item_done(r#"{"type":"response.output_item.done","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
#[case::arguments_done(r#"{"type":"response.function_call_arguments.done","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
#[case::summary_text_done(r#"{"type":"response.reasoning_summary_text.done","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
#[case::summary_part_done(r#"{"type":"response.reasoning_summary_part.done","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
#[case::unknown_event(r#"{"type":"other","delta":"<EMAIL_ADDRESS_1>"}"#, TOKENS, None, &[])]
fn response_event_unmasking(
    #[case] message: &str,
    #[case] stored: &str,
    #[case] expected: Option<&str>,
    #[case] unmasked_texts: &[&str],
) {
    let unmask_calls = Arc::new(Mutex::new(Vec::new()));
    let mask_calls = Arc::new(Mutex::new(Vec::new()));
    let first = recording_unmask(0, &unmask_calls);
    let second = recording_unmask(1, &unmask_calls);
    let guardrails = [
        InputGuardrail {
            callback: masker(0, Vec::new(), &mask_calls),
            unmask_pii_text: &first,
        },
        InputGuardrail {
            callback: masker(1, Vec::new(), &mask_calls),
            unmask_pii_text: &second,
        },
    ];
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &guardrails,
        output_guardrail_callbacks: &[],
        authorized_model: None,
        request_defaults: None,
    };
    let data = request_data(stored);
    let unmasked = unmask_response_event(&policy, &data, message).unwrap();
    assert_eq!(
        matches!(unmasked, Cow::Borrowed(text) if text == message),
        expected.is_none()
    );
    assert_eq!(parsed(&unmasked), parsed(expected.unwrap_or(message)));
    let expected_calls: Vec<(usize, String)> = unmasked_texts
        .iter()
        .map(|text| (0_usize, (*text).to_owned()))
        .collect();
    assert_eq!(*lock(&unmask_calls), expected_calls);
    assert!(lock(&mask_calls).is_empty());
    let unguarded = FramePolicy::<Masker> {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &[],
        authorized_model: None,
        request_defaults: None,
    };
    let untouched = unmask_response_event(&unguarded, &data, message).unwrap();
    assert!(matches!(untouched, Cow::Borrowed(text) if text == message));
}

#[rstest]
#[case::changed(
    r#"{"type":"response.completed","response":{"output":[{"arguments":"secret args","summary":[{"text":"secret sum"},{"text":5},"s"],"content":[{"text":"secret text"},{"text":"plain"}]},{"arguments":5,"summary":"x","content":"x"},"item"]}}"#,
    &["secret args", "secret sum", "secret text", "plain"],
    Some(r#"{"type":"response.completed","response":{"output":[{"arguments":"[M] args","summary":[{"text":"[M] sum"},{"text":5},"s"],"content":[{"text":"[M] text"},{"text":"plain"}]},{"arguments":5,"summary":"x","content":"x"},"item"]}}"#)
)]
#[case::arguments_only(r#"{"type":"response.completed","response":{"output":[{"arguments":"secret"}]}}"#, &["secret"], Some(r#"{"type":"response.completed","response":{"output":[{"arguments":"[M]"}]}}"#))]
#[case::summary_only(r#"{"type":"response.completed","response":{"output":[{"summary":[{"text":"secret"}]}]}}"#, &["secret"], Some(r#"{"type":"response.completed","response":{"output":[{"summary":[{"text":"[M]"}]}]}}"#))]
#[case::equal_values(r#"{"type": "response.completed", "response": {"output": [{"arguments": "a", "summary": [{"text": "b"}], "content": [{"text": "c"}]}]}}"#, &["a", "b", "c"], None)]
#[case::not_json("not-json", &[], None)]
#[case::other_event(r#"{"type":"response.output_text.delta","delta":"secret"}"#, &[], None)]
#[case::missing_response(r#"{"type":"response.completed"}"#, &[], None)]
#[case::text_response(r#"{"type":"response.completed","response":"secret"}"#, &[], None)]
#[case::no_output_text(r#"{"type":"response.completed","response":{"output":[{"type":"message"}]}}"#, &[], None)]
#[tokio::test]
async fn completed_output_masking(
    #[case] message: &str,
    #[case] first_texts: &[&str],
    #[case] expected: Option<&str>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let outputs = [
        masker(0, vec![("secret", "[S]")], &calls),
        masker(1, vec![("[S]", "[M]")], &calls),
    ];
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &outputs,
        authorized_model: None,
        request_defaults: None,
    };
    let data = request_data("{}");
    let masked = mask_response_completed(&policy, &data, message)
        .await
        .unwrap();
    assert_eq!(
        matches!(masked, Cow::Borrowed(text) if text == message),
        expected.is_none()
    );
    assert_eq!(parsed(&masked), parsed(expected.unwrap_or(message)));
    let second_texts = first_texts
        .iter()
        .map(|text| (1_usize, text.replace("secret", "[S]")));
    let expected_calls: Vec<MaskCall> = first_texts
        .iter()
        .map(|text| (0_usize, (*text).to_owned()))
        .chain(second_texts)
        .map(|(guard, text)| MaskCall {
            guard,
            text,
            output_parse_pii: false,
            metadata: None,
        })
        .collect();
    assert_eq!(*lock(&calls), expected_calls);
    let unguarded = FramePolicy::<Masker> {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &[],
        authorized_model: None,
        request_defaults: None,
    };
    let untouched = mask_response_completed(&unguarded, &data, message)
        .await
        .unwrap();
    assert!(matches!(untouched, Cow::Borrowed(text) if text == message));
}

#[rstest]
#[case::array("[1,2,3]")]
#[case::text(r#""response.create""#)]
#[case::number("5")]
#[case::null("null")]
#[tokio::test]
async fn non_object_masking_frames(#[case] message: &str) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let guardrails = [InputGuardrail {
        callback: masker(0, Vec::new(), &calls),
        unmask_pii_text: &echo,
    }];
    let outputs = [masker(1, Vec::new(), &calls)];
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &guardrails,
        output_guardrail_callbacks: &outputs,
        authorized_model: None,
        request_defaults: None,
    };
    let mut data = request_data(TOKENS);
    let created = mask_response_create(&policy, &mut data, message).await;
    assert!(matches!(created, Err(FramePolicyError::NotAnObject)));
    let unmasked = unmask_response_event(&policy, &data, message);
    assert!(matches!(unmasked, Err(FramePolicyError::NotAnObject)));
    let completed = mask_response_completed(&policy, &data, message).await;
    assert!(matches!(completed, Err(FramePolicyError::NotAnObject)));
    assert!(lock(&calls).is_empty());
}

#[rstest]
#[case::lone_surrogate(
    r#"{"type":"response.completed","model":"client","input":"\ud800","response":{"output":[{"content":[{"text":"<EMAIL_ADDRESS_1>"}]}]}}"#
)]
#[case::not_a_number(r#"{"type":"response.completed","temperature":NaN}"#)]
#[case::leading_whitespace(" \n{\"type\":\"response.completed\",")]
#[tokio::test]
async fn unreadable_object_frames(#[case] message: &str) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let guardrails = [InputGuardrail {
        callback: masker(0, Vec::new(), &calls),
        unmask_pii_text: &echo,
    }];
    let outputs = [masker(1, Vec::new(), &calls)];
    let policy = FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: &guardrails,
        output_guardrail_callbacks: &outputs,
        authorized_model: Some("gpt-4o"),
        request_defaults: None,
    };
    let created_frame = message.replace("response.completed", "response.create");
    let mut data = request_data(TOKENS);
    let created = mask_response_create(&policy, &mut data, &created_frame).await;
    assert!(matches!(created, Err(FramePolicyError::Unreadable { .. })));
    let unmasked = unmask_response_event(&policy, &data, message);
    assert!(matches!(unmasked, Err(FramePolicyError::Unreadable { .. })));
    let completed = mask_response_completed(&policy, &data, message).await;
    assert!(matches!(
        completed,
        Err(FramePolicyError::Unreadable { .. })
    ));
    assert!(lock(&calls).is_empty());
}

#[rstest]
#[case::rejected(CREATE, Verdict::RateLimited, false, Ok(false), 1)]
#[case::send_fails(CREATE, Verdict::RateLimited, true, Ok(false), 1)]
#[case::other_error(CREATE, Verdict::Broken, false, Err("boom"), 0)]
#[case::accepted(CREATE, Verdict::Accept, false, Ok(true), 0)]
#[case::not_charged(
    r#"{"type":"response.completed"}"#,
    Verdict::RateLimited,
    false,
    Ok(true),
    0
)]
#[tokio::test]
async fn quota_rejection(
    #[case] message: &str,
    #[case] second: Verdict,
    #[case] refuse: bool,
    #[case] expected: Result<bool, &str>,
    #[case] attempts: usize,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callbacks = [
        quota("one", Verdict::Accept, &calls),
        quota("two", second, &calls),
    ];
    let user = key();
    let policy = FramePolicy::<Masker> {
        quota_callbacks: &callbacks,
        user_api_key_dict: Some(&user),
        guardrail_callbacks: &[],
        output_guardrail_callbacks: &[],
        authorized_model: Some("gpt-4o"),
        request_defaults: None,
    };
    let client = Client {
        refuse,
        sent: Mutex::new(Vec::new()),
    };
    let outcome = enforce_or_reject_frame(&policy, &client, message).await;
    let observed = outcome.map_err(|error| class_message(&error));
    assert_eq!(observed, expected.map_err(str::to_owned));
    let rejection =
        r#"{"type":"error","error":{"type":"rate_limit_exceeded","message":"quota exceeded"}}"#;
    let sent: Vec<&str> = repeat_n(rejection, attempts).collect();
    assert_eq!(*lock(&client.sent), sent);
    let charged = if message == CREATE { 2 } else { 0 };
    assert_eq!(lock(&calls).len(), charged);
}
