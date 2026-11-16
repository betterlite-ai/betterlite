use serde_json::{Value, json};

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::list::{list_request, list_response};
use crate::videos::segment::path_url;
use crate::videos::status::status_response;

use super::support::{
    API_BASE, MODEL, SLASH_BASE, fixture, model_of, options, video_body, video_of,
};

const LIST_FIXTURE: &str = r#"{
    "object": "list",
    "data": [
        {"id": "vid-1", "object": "video", "status": "completed", "model": "sora-2", "prompt": "a cat"},
        {"id": "vid-2", "object": "video", "status": "queued", "model": null, "vendor": {"n": 1}}
    ],
    "first_id": "vid-1",
    "last_id": "vid-2",
    "has_more": true,
    "vendor_page": 3
}"#;

const STATUS_FIXTURE: &str = r#"{
    "id": "vid-3",
    "object": "video",
    "status": "completed",
    "seconds": "4",
    "usage": {"duration_seconds": 99, "other": true}
}"#;

fn list_request_case() {
    let cursor = encode_video_id_with_provider("vid-cursor", "openai", Some(MODEL));
    let (url, params) = list_request(
        SLASH_BASE,
        Some(&cursor),
        Some(0),
        Some("asc"),
        &VideoOptions::new(),
    );
    assert_eq!(url, SLASH_BASE);
    assert_eq!(
        Value::Object(params),
        fixture(r#"{"after": "vid-cursor", "limit": "0", "order": "asc"}"#)
    );
    let (defaults_url, defaults) = list_request(API_BASE, None, None, None, &VideoOptions::new());
    assert_eq!(defaults_url, API_BASE);
    assert!(defaults.is_empty());
    let extra = options(&[("after", json!("overridden")), ("limit", json!("9"))]);
    let (extra_url, merged) = list_request(API_BASE, Some(&cursor), Some(5), None, &extra);
    assert_eq!(extra_url, API_BASE);
    assert_eq!(
        Value::Object(merged),
        fixture(r#"{"after": "overridden", "limit": "9"}"#)
    );
}

fn list_wrap_case() {
    let decoded = list_response(LIST_FIXTURE.as_bytes(), Some("openai")).unwrap();
    let first = encode_video_id_with_provider("vid-1", "openai", Some(MODEL));
    let last = encode_video_id_with_provider("vid-2", "openai", None);
    assert_eq!(decoded.first_id, Some(Some(first.clone())));
    assert_eq!(decoded.last_id, Some(Some(last.clone())));
    let ids: Vec<Option<String>> = decoded.data.iter().map(|item| item.id.clone()).collect();
    assert_eq!(ids, [Some(first.clone()), Some(last.clone())]);
    assert_eq!(model_of(&first).as_deref(), Some(MODEL));
    // the codec decodes the empty model segment a None wrap writes as an empty string
    assert_eq!(model_of(&last).as_deref(), Some(""));
    let second = &decoded.data[1];
    assert_eq!(second.model, Some(None));
    let vendor = Value::Object(options(&[("n", json!(1_i64))]));
    assert_eq!(second.fields.get("vendor"), Some(&vendor));
    let first_item = decoded.data.first().unwrap();
    assert_eq!(first_item.fields.get("prompt"), Some(&json!("a cat")));
    assert_eq!(decoded.object.as_deref(), Some("list"));
    assert_eq!(decoded.has_more, Some(true));
    assert_eq!(decoded.fields.get("vendor_page"), Some(&json!(3_i64)));
}

fn list_typed_fields_case() {
    let decoded = list_response(LIST_FIXTURE.as_bytes(), None).unwrap();
    assert_eq!(decoded.object.as_deref(), Some("list"));
    assert_eq!(decoded.has_more, Some(true));
    let models: Vec<_> = decoded.data.iter().map(|item| item.model.clone()).collect();
    assert_eq!(models, [Some(Some(MODEL.to_owned())), Some(None)]);
    let prompts: Vec<_> = decoded
        .data
        .iter()
        .map(|item| item.fields.get("prompt").cloned())
        .collect();
    assert_eq!(prompts, [Some(json!("a cat")), None]);
}

fn list_empty_provider_case() {
    let plain = list_response(LIST_FIXTURE.as_bytes(), Some("")).unwrap();
    assert_eq!(plain.first_id, Some(Some("vid-1".to_owned())));
    assert_eq!(serde_json::to_value(&plain).unwrap(), fixture(LIST_FIXTURE));
}

fn list_empty_data_case() {
    let cursors = list_response(
        br#"{"data": [], "first_id": "vid-1", "last_id": "vid-2"}"#,
        Some("openai"),
    )
    .unwrap();
    let first = cursors.first_id.unwrap().unwrap();
    let last = cursors.last_id.unwrap().unwrap();
    assert_eq!(video_of(&first), "vid-1");
    assert_eq!(model_of(&first).as_deref(), Some(""));
    assert_eq!(video_of(&last), "vid-2");
    assert!(cursors.data.is_empty());
}

fn list_native_case() {
    let native = list_response(
        br#"{"first_id": "vid-1", "object": "list"}"#,
        Some("openai"),
    )
    .unwrap();
    assert_eq!(native.first_id, Some(Some("vid-1".to_owned())));
    assert_eq!(native.object.as_deref(), Some("list"));
    assert_eq!(
        serde_json::to_value(&native).unwrap(),
        fixture(r#"{"data": [], "first_id": "vid-1", "object": "list"}"#)
    );
}

fn list_cursor_forms_case() {
    let source = r#"{"data": [], "first_id": null, "last_id": ""}"#;
    let cursors = list_response(source.as_bytes(), Some("openai")).unwrap();
    assert_eq!(cursors.first_id, Some(None));
    assert_eq!(cursors.last_id, Some(Some(String::new())));
    assert_eq!(cursors.has_more, None);
    assert_eq!(serde_json::to_value(&cursors).unwrap(), fixture(source));
    let absent = list_response(br#"{"data": []}"#, Some("openai")).unwrap();
    assert_eq!(absent.first_id, None);
    assert_eq!(
        serde_json::to_value(&absent).unwrap(),
        fixture(r#"{"data": []}"#)
    );
}

fn list_item_forms_case() {
    let missing = br#"{"data": [{"object": "video"}]}"#;
    let without_id = list_response(missing, Some("openai")).unwrap();
    assert_eq!(without_id.data.first().unwrap().id, None);
    let bare = r#"{"data": [{"id": "vid-9", "object": "video"}]}"#;
    let no_model = list_response(bare.as_bytes(), None).unwrap();
    assert_eq!(no_model.data.first().unwrap().model, None);
    assert_eq!(serde_json::to_value(&no_model).unwrap(), fixture(bare));
    let nulled = br#"{"data": [{"id": null, "object": "video"}]}"#;
    let null_id = list_response(nulled, Some("openai")).unwrap();
    assert_eq!(null_id.data.first().unwrap().id, None);
    assert_eq!(
        serde_json::to_value(&null_id).unwrap(),
        fixture(r#"{"data": [{"object": "video"}]}"#)
    );
}

fn list_rejects_case() {
    let number_model = br#"{"data": [{"id": "vid-9", "model": 5}]}"#;
    assert!(matches!(
        list_response(number_model, None),
        Err(VideoError::Reply { .. })
    ));
    let number_cursor = br#"{"data": [], "first_id": 7}"#;
    assert!(matches!(
        list_response(number_cursor, Some("openai")),
        Err(VideoError::Reply { .. })
    ));
    let number_id = br#"{"data": [{"id": 7}]}"#;
    assert!(matches!(
        list_response(number_id, None),
        Err(VideoError::Reply { .. })
    ));
    let number_data = br#"{"data": 3}"#;
    assert!(matches!(
        list_response(number_data, Some("openai")),
        Err(VideoError::Reply { .. })
    ));
}

fn status_case() {
    let encoded = encode_video_id_with_provider("vid-3", "openai", Some(MODEL));
    assert_eq!(
        path_url(&encoded, SLASH_BASE, "").unwrap(),
        "https://mock.example/v1/videos/vid-3"
    );
    let retrieved = status_response(STATUS_FIXTURE.as_bytes(), Some("openai")).unwrap();
    assert_eq!(video_of(&retrieved.id), "vid-3");
    assert_eq!(model_of(&retrieved.id).as_deref(), Some(""));
    let usage = options(&[("duration_seconds", json!(99_i64)), ("other", json!(true))]);
    assert_eq!(retrieved.usage, Some(usage));
    let wrapped = status_response(&video_body("vid-3", None), Some("openai")).unwrap();
    assert_eq!(video_of(&wrapped.id), "vid-3");
    assert_eq!(wrapped.usage, None);
    let native = status_response(&video_body("vid-3", None), None).unwrap();
    assert_eq!(native.id, "vid-3");
    let empty = status_response(&video_body("", None), Some("openai")).unwrap();
    assert_eq!(empty.id, "");
}

pub(super) fn list_and_status_case() {
    list_request_case();
    list_wrap_case();
    list_typed_fields_case();
    list_empty_provider_case();
    list_empty_data_case();
    list_native_case();
    list_cursor_forms_case();
    list_item_forms_case();
    list_rejects_case();
    status_case();
}
