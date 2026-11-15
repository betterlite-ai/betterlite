use serde_json::{Value, json};

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::VideoOptions;
use crate::videos::list::{list_request, list_response};
use crate::videos::segment::path_url;
use crate::videos::status::status_response;

use super::support::{API_BASE, MODEL, SLASH_BASE, body, model_of, options, video_body, video_of};

fn list_fixture() -> Value {
    json!({
        "object": "list",
        "data": [
            {"id": "vid-1", "object": "video", "status": "completed", "model": MODEL, "prompt": "a cat"},
            {"id": "vid-2", "object": "video", "status": "queued", "model": null, "vendor": {"n": 1_i64}}
        ],
        "first_id": "vid-1",
        "last_id": "vid-2",
        "has_more": true,
        "vendor_page": 3_i64
    })
}

fn list_request_fields_case() {
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
        json!({"after": "vid-cursor", "limit": "0", "order": "asc"})
    );
}

fn list_request_defaults_case() {
    let (url, params) = list_request(API_BASE, None, None, None, &VideoOptions::new());
    assert_eq!(url, API_BASE);
    assert!(params.is_empty());
}

fn list_request_extra_case() {
    let cursor = encode_video_id_with_provider("vid-cursor", "openai", Some(MODEL));
    let extra = options(&[("after", json!("overridden")), ("limit", json!("9"))]);
    let (_, params) = list_request(API_BASE, Some(&cursor), Some(5), None, &extra);
    assert_eq!(
        Value::Object(params),
        json!({"after": "overridden", "limit": "9"})
    );
}

fn list_wrap_case() {
    let decoded = list_response(&body(&list_fixture()), Some("openai")).unwrap();
    let first = encode_video_id_with_provider("vid-1", "openai", Some(MODEL));
    let last = encode_video_id_with_provider("vid-2", "openai", None);
    assert_eq!(decoded.first_id, Some(Some(first.clone())));
    assert_eq!(decoded.last_id, Some(Some(last.clone())));
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        json!({
            "object": "list",
            "data": [
                {"id": first, "object": "video", "status": "completed", "model": MODEL, "prompt": "a cat"},
                {"id": last, "object": "video", "status": "queued", "model": null, "vendor": {"n": 1_i64}}
            ],
            "first_id": first,
            "last_id": last,
            "has_more": true,
            "vendor_page": 3_i64
        })
    );
    assert_eq!(model_of(&first).as_deref(), Some(MODEL));
    assert_eq!(model_of(&last), None);
}

fn list_typed_fields_case() {
    let decoded = list_response(&body(&list_fixture()), None).unwrap();
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
    let reply = list_fixture();
    let plain = list_response(&body(&reply), Some("")).unwrap();
    assert_eq!(plain.first_id, Some(Some("vid-1".to_owned())));
    assert_eq!(serde_json::to_value(&plain).unwrap(), reply);
}

fn list_empty_data_case() {
    let source = json!({"data": [], "first_id": "vid-1", "last_id": "vid-2"});
    let cursors = list_response(&body(&source), Some("openai")).unwrap();
    let first = cursors.first_id.unwrap().unwrap();
    let last = cursors.last_id.unwrap().unwrap();
    assert_eq!(video_of(&first), "vid-1");
    assert_eq!(model_of(&first), None);
    assert_eq!(video_of(&last), "vid-2");
    assert!(cursors.data.is_empty());
}

fn list_native_case() {
    let source = json!({"first_id": "vid-1", "object": "list"});
    let native = list_response(&body(&source), Some("openai")).unwrap();
    assert_eq!(native.first_id, Some(Some("vid-1".to_owned())));
    assert_eq!(native.object.as_deref(), Some("list"));
    assert_eq!(
        serde_json::to_value(&native).unwrap(),
        json!({"data": [], "first_id": "vid-1", "object": "list"})
    );
}

fn list_cursor_forms_case() {
    let source = json!({"data": [], "first_id": null, "last_id": ""});
    let null_cursor = list_response(&body(&source), Some("openai")).unwrap();
    assert_eq!(null_cursor.first_id, Some(None));
    assert_eq!(null_cursor.last_id, Some(Some(String::new())));
    assert_eq!(null_cursor.has_more, None);
    assert_eq!(serde_json::to_value(&null_cursor).unwrap(), source);
    let absent = list_response(&body(&json!({"data": []})), Some("openai")).unwrap();
    assert_eq!(absent.first_id, None);
    assert_eq!(serde_json::to_value(&absent).unwrap(), json!({"data": []}));
}

fn list_item_forms_case() {
    let missing = json!({"data": [{"object": "video"}]});
    let without_id = list_response(&body(&missing), Some("openai")).unwrap();
    assert_eq!(without_id.data.first().unwrap().id, None);
    let bare = json!({"data": [{"id": "vid-9", "object": "video"}]});
    let no_model = list_response(&body(&bare), None).unwrap();
    assert_eq!(no_model.data.first().unwrap().model, None);
    assert_eq!(serde_json::to_value(&no_model).unwrap(), bare);
    let nulled = json!({"data": [{"id": null, "object": "video"}]});
    let null_id = list_response(&body(&nulled), Some("openai")).unwrap();
    assert_eq!(null_id.data.first().unwrap().id, None);
    assert_eq!(
        serde_json::to_value(&null_id).unwrap(),
        json!({"data": [{"object": "video"}]})
    );
}

fn list_rejects_case() {
    let number_model = json!({"data": [{"id": "vid-9", "model": 5_i64}]});
    assert!(list_response(&body(&number_model), None).is_err());
    let number_cursor = json!({"data": [], "first_id": 7_i64});
    assert!(list_response(&body(&number_cursor), Some("openai")).is_err());
    let number_id = json!({"data": [{"id": 7_i64}]});
    assert!(list_response(&body(&number_id), None).is_err());
    let number_data = json!({"data": 3_i64});
    assert!(list_response(&body(&number_data), Some("openai")).is_err());
}

fn status_case() {
    let encoded = encode_video_id_with_provider("vid-3", "openai", Some(MODEL));
    assert_eq!(
        path_url(&encoded, SLASH_BASE, "").unwrap(),
        "https://mock.example/v1/videos/vid-3"
    );
    let reply = json!({
        "id": "vid-3",
        "object": "video",
        "status": "completed",
        "seconds": "4",
        "usage": {"duration_seconds": 99_i64, "other": true}
    });
    let retrieved = status_response(&body(&reply), Some("openai")).unwrap();
    assert_eq!(video_of(&retrieved.id), "vid-3");
    assert_eq!(model_of(&retrieved.id), None);
    assert_eq!(
        Value::Object(retrieved.usage.unwrap()),
        json!({"duration_seconds": 99_i64, "other": true})
    );
}

fn status_forms_case() {
    let wrapped = status_response(&video_body("vid-3", None), Some("openai")).unwrap();
    assert_eq!(video_of(&wrapped.id), "vid-3");
    assert_eq!(wrapped.usage, None);
    let native = status_response(&video_body("vid-3", None), None).unwrap();
    assert_eq!(native.id, "vid-3");
    let empty = status_response(&video_body("", None), Some("openai")).unwrap();
    assert_eq!(empty.id, "");
}

pub(super) fn list_and_status_case() {
    list_request_fields_case();
    list_request_defaults_case();
    list_request_extra_case();
    list_wrap_case();
    list_typed_fields_case();
    list_empty_provider_case();
    list_empty_data_case();
    list_native_case();
    list_cursor_forms_case();
    list_item_forms_case();
    list_rejects_case();
    status_case();
    status_forms_case();
}
