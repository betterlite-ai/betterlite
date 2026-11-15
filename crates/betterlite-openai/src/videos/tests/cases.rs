use rstest::rstest;
use serde_json::{Value, json};

use betterlite_http::error::HttpError;

use crate::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_character_id, encode_video_id_with_provider,
};
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::videos::VideoOptions;
use crate::videos::config::{
    DEFAULT_API_BASE, MULTIPART_FORM_DATA, SUPPORTED_PARAMS, VideoCredentials, complete_url,
    map_params, validate_environment,
};
use crate::videos::content::{content_request, content_response};
use crate::videos::create::{VideoCreate, create_request, create_response};
use crate::videos::delete::{delete_request, delete_response};
use crate::videos::error::{VideoError, get_error_class};
use crate::videos::list::{list_request, list_response};
use crate::videos::remix::{remix_request, remix_response};
use crate::videos::reply::VideoObject;
use crate::videos::status::{status_request, status_response};

use super::support::{body, credentials, exposed, headers, options, secret_headers, video_create};

const MODEL: &str = "sora-2";
const PROMPT: &str = "make a video";
const API_BASE: &str = "https://mock.example/v1/videos";

fn video_of(encoded: &str) -> String {
    decode_video_id_with_provider(encoded).video_id
}

fn model_of(encoded: &str) -> Option<String> {
    decode_video_id_with_provider(encoded).model_id
}

fn seconds_video(seconds: Option<Value>) -> VideoObject {
    let mut reply = json!({"id": "vid-7", "object": "video", "status": "completed"});
    if let Some(seconds) = seconds {
        reply["seconds"] = seconds;
    }
    create_response(MODEL, &body(&reply), Some("openai")).unwrap()
}

fn create_multipart_case() {
    let character = encode_character_id("char-1", "openai", Some(MODEL));
    let given_json = json!({
        "model": "option-model",
        "prompt": "option-prompt",
        "extra_headers": {"x-option": "1"},
        "input_reference": "option-reference",
        "seconds": "4",
        "size": "720x1280",
        "user": "user-1",
        "characters": [{"id": character, "role": "hero"}],
        "vendor_flag": true,
    });
    let given = video_create(given_json.clone());
    let form = create_request(
        MODEL,
        PROMPT,
        API_BASE,
        &given,
        Some(MediaBody::Text("https://example.invalid/a.png")),
    );
    assert_eq!(
        Value::Object(form.fields.clone()),
        json!({
            "model": MODEL,
            "prompt": PROMPT,
            "seconds": "4",
            "size": "720x1280",
            "user": "user-1",
            "characters": [{"id": "char-1", "role": "hero"}],
            "vendor_flag": true,
        })
    );
    assert_eq!(form.files.len(), 1);
    assert_eq!(form.files[0].field, "input_reference");
    assert_eq!(form.files[0].filename, "input_reference.png");
    assert_eq!(form.files[0].content_type, "image/png");
    assert!(matches!(
        &form.files[0].body,
        MediaBody::Text(text) if *text == "https://example.invalid/a.png"
    ));
    assert_eq!(form.api_base, API_BASE);
    assert_eq!(
        SUPPORTED_PARAMS,
        [
            "model",
            "prompt",
            "input_reference",
            "seconds",
            "size",
            "characters",
            "user",
            "extra_headers"
        ]
    );
    assert!(MULTIPART_FORM_DATA);
    let mut mapped = VideoOptions::new();
    map_params(&given, &mut mapped);
    assert_eq!(Value::Object(mapped), given_json);
    assert!(serde_json::from_value::<VideoCreate>(json!({"seconds": 4})).is_err());
    assert!(serde_json::from_value::<VideoCreate>(json!({"characters": [{"id": 1}]})).is_err());
}

fn input_reference_case() {
    let png = create_request(
        MODEL,
        PROMPT,
        API_BASE,
        &VideoCreate::default(),
        Some(MediaBody::Bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR")),
    );
    assert_eq!(png.files.len(), 1);
    assert_eq!(png.files[0].filename, "input_reference.png");
    assert_eq!(png.files[0].content_type, "image/png");
    assert!(matches!(
        &png.files[0].body,
        MediaBody::Bytes(bytes) if *bytes == b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".as_slice()
    ));

    let jpeg = create_request(
        MODEL,
        PROMPT,
        API_BASE,
        &VideoCreate::default(),
        Some(MediaBody::Bytes(b"\xff\xd8\xff\xe0\x00\x10JFIF\x00")),
    );
    assert_eq!(jpeg.files[0].content_type, "image/jpeg");

    let empty = create_request(
        MODEL,
        PROMPT,
        API_BASE,
        &VideoCreate::default(),
        Some(MediaBody::Text("")),
    );
    assert_eq!(empty.files[0].content_type, "image/png");
    assert!(matches!(&empty.files[0].body, MediaBody::Text(text) if text.is_empty()));

    let none = create_request(MODEL, PROMPT, API_BASE, &VideoCreate::default(), None);
    assert!(none.files.is_empty());

    let given = video_create(json!({"input_reference": "option-reference"}));
    let dropped = create_request(MODEL, PROMPT, API_BASE, &given, None);
    assert!(!dropped.fields.contains_key("input_reference"));
    assert!(dropped.files.is_empty());
}

fn list_requests() {
    let after = encode_video_id_with_provider("vid-cursor", "openai", Some(MODEL));
    let (url, params) = list_request(
        "https://mock.example/v1/videos/",
        Some(&after),
        Some(0),
        Some("asc"),
        &VideoOptions::new(),
    );
    assert_eq!(url, "https://mock.example/v1/videos/");
    assert_eq!(
        Value::Object(params),
        json!({"after": "vid-cursor", "limit": "0", "order": "asc"})
    );

    let (url, params) = list_request(
        "https://mock.example/v1/videos",
        None,
        None,
        None,
        &VideoOptions::new(),
    );
    assert_eq!(url, "https://mock.example/v1/videos");
    assert!(params.is_empty());

    let extra = options(&[("after", json!("overridden")), ("limit", json!("9"))]);
    let (_, params) = list_request(
        "https://mock.example/v1/videos",
        Some(&after),
        Some(5),
        None,
        &extra,
    );
    assert_eq!(
        Value::Object(params),
        json!({"after": "overridden", "limit": "9"})
    );
}

fn list_replies() {
    let reply = json!({
        "object": "list",
        "data": [
            {"id": "vid-1", "object": "video", "status": "completed", "model": MODEL, "prompt": "a cat"},
            {"id": "vid-2", "object": "video", "status": "queued", "model": null, "vendor": {"n": 1}}
        ],
        "first_id": "vid-1",
        "last_id": "vid-2",
        "has_more": true,
        "vendor_page": 3
    });
    let decoded = list_response(&body(&reply), Some("openai")).unwrap();
    assert_eq!(decoded.object.as_deref(), Some("list"));
    assert_eq!(decoded.has_more, Some(true));
    assert_eq!(decoded.data[0].model, Some(Some(MODEL.to_owned())));
    assert_eq!(decoded.data[1].model, Some(None));
    assert_eq!(decoded.data[0].fields.get("prompt"), Some(&json!("a cat")));
    let wrapped = serde_json::to_value(&decoded).unwrap();
    let first_id = encode_video_id_with_provider("vid-1", "openai", Some(MODEL));
    let last_id = encode_video_id_with_provider("vid-2", "openai", None);
    assert_eq!(decoded.first_id, Some(Some(first_id.clone())));
    assert_eq!(decoded.last_id, Some(Some(last_id.clone())));
    assert_eq!(
        wrapped,
        json!({
            "object": "list",
            "data": [
                {"id": first_id, "object": "video", "status": "completed", "model": MODEL, "prompt": "a cat"},
                {"id": last_id, "object": "video", "status": "queued", "model": null, "vendor": {"n": 1}}
            ],
            "first_id": first_id,
            "last_id": last_id,
            "has_more": true,
            "vendor_page": 3
        })
    );
    assert_eq!(model_of(&first_id).as_deref(), Some(MODEL));
    assert_eq!(model_of(&last_id), None);

    let empty = json!({"data": [], "first_id": "vid-1", "last_id": "vid-2"});
    let cursors = list_response(&body(&empty), Some("openai")).unwrap();
    let rendered = serde_json::to_value(&cursors).unwrap();
    assert_eq!(model_of(rendered["first_id"].as_str().unwrap()), None);
    assert_eq!(video_of(rendered["first_id"].as_str().unwrap()), "vid-1");
    assert_eq!(video_of(rendered["last_id"].as_str().unwrap()), "vid-2");
    assert!(cursors.data.is_empty());

    let native = list_response(
        &body(&json!({"first_id": "vid-1", "object": "list"})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(native.first_id, Some(Some("vid-1".to_owned())));
    assert_eq!(native.object.as_deref(), Some("list"));
    assert_eq!(
        serde_json::to_value(&native).unwrap(),
        json!({"data": [], "first_id": "vid-1", "object": "list"})
    );

    let null_cursor = list_response(
        &body(&json!({"data": [], "first_id": null, "last_id": ""})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(null_cursor.first_id, Some(None));
    assert_eq!(null_cursor.last_id, Some(Some(String::new())));
    assert_eq!(null_cursor.has_more, None);
    assert_eq!(
        serde_json::to_value(&null_cursor).unwrap(),
        json!({"data": [], "first_id": null, "last_id": ""})
    );

    let absent_cursor = list_response(&body(&json!({"data": []})), Some("openai")).unwrap();
    assert_eq!(absent_cursor.first_id, None);
    assert_eq!(
        serde_json::to_value(&absent_cursor).unwrap(),
        json!({"data": []})
    );

    let item_without_id = list_response(
        &body(&json!({"data": [{"object": "video"}]})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(item_without_id.data[0].id, None);

    let no_model = list_response(
        &body(&json!({"data": [{"id": "vid-9", "object": "video"}]})),
        None,
    )
    .unwrap();
    assert_eq!(no_model.data[0].model, None);
    assert_eq!(
        serde_json::to_value(&no_model).unwrap(),
        json!({"data": [{"id": "vid-9", "object": "video"}]})
    );
    assert!(list_response(&body(&json!({"data": [{"id": "vid-9", "model": 5}]})), None).is_err());
    assert!(list_response(&body(&json!({"data": [], "first_id": 7})), Some("openai")).is_err());
    assert!(list_response(&body(&json!({"data": [{"id": 7}]})), None).is_err());
    assert!(list_response(&body(&json!({"data": 3})), Some("openai")).is_err());

    let null_id = list_response(
        &body(&json!({"data": [{"id": null, "object": "video"}]})),
        None,
    )
    .unwrap();
    assert_eq!(null_id.data[0].id, None);
    assert_eq!(
        serde_json::to_value(&null_id).unwrap(),
        json!({"data": [{"object": "video"}]})
    );

    let plain = list_response(&body(&reply), Some("")).unwrap();
    assert_eq!(plain.data[0].id.as_deref(), Some("vid-1"));
    assert_eq!(plain.first_id, Some(Some("vid-1".to_owned())));
    assert_eq!(plain.data[1].model, Some(None));
    assert_eq!(serde_json::to_value(&plain).unwrap(), reply);
}

fn status_calls() {
    let encoded = encode_video_id_with_provider("vid-3", "openai", Some(MODEL));
    let (url, params) = status_request(&encoded, "https://mock.example/v1/videos/");
    assert_eq!(url, "https://mock.example/v1/videos/vid-3");
    assert!(params.is_empty());

    let reply = json!({
        "id": "vid-3",
        "object": "video",
        "status": "completed",
        "seconds": "4",
        "usage": {"duration_seconds": 99, "other": true}
    });
    let retrieved = status_response(&body(&reply), Some("openai")).unwrap();
    assert_eq!(video_of(&retrieved.id), "vid-3");
    assert_eq!(model_of(&retrieved.id), None);
    assert_eq!(
        Value::Object(retrieved.usage.unwrap()),
        json!({"duration_seconds": 99, "other": true})
    );

    let plain = status_response(
        &body(&json!({"id": "vid-3", "object": "video", "status": "completed"})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(plain.id, "vid-3");
    assert_eq!(plain.usage, None);

    let empty = status_response(
        &body(&json!({"id": "", "object": "video", "status": "completed"})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(empty.id, "");
}

fn list_and_status_case() {
    list_requests();
    list_replies();
    status_calls();
}

fn remix_case() {
    let encoded = encode_video_id_with_provider("vid/one two", "openai", Some(MODEL));
    let (url, data) = remix_request(
        &encoded,
        "make it blue",
        "https://mock.example/v1/videos/",
        &VideoOptions::new(),
    )
    .unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid%2Fone%20two/remix");
    assert_eq!(Value::Object(data), json!({"prompt": "make it blue"}));

    let extra = options(&[("prompt", json!("overwritten")), ("seconds", json!("4"))]);
    let (_, merged) = remix_request(
        &encoded,
        "make it blue",
        "https://mock.example/v1/videos",
        &extra,
    )
    .unwrap();
    assert_eq!(
        Value::Object(merged),
        json!({"prompt": "overwritten", "seconds": "4"})
    );

    let reply = json!({
        "id": "vid-4",
        "object": "video",
        "status": "queued",
        "seconds": "4",
        "usage": {"ignored": true}
    });
    let remixed = remix_response(&body(&reply), Some("openai")).unwrap();
    assert_eq!(video_of(&remixed.id), "vid-4");
    assert_eq!(model_of(&remixed.id), None);
    assert_eq!(
        Value::Object(remixed.usage.unwrap()),
        json!({"duration_seconds": 4.0})
    );

    let native = remix_response(
        &body(&json!({"id": "vid-4", "object": "video", "status": "queued", "seconds": "abc"})),
        None,
    )
    .unwrap();
    assert_eq!(native.id, "vid-4");
    assert!(native.usage.unwrap().is_empty());

    let numeric = remix_response(
        &body(&json!({"id": "vid-4", "object": "video", "status": "queued", "seconds": 0})),
        Some("openai"),
    )
    .unwrap();
    assert!(numeric.usage.unwrap().is_empty());
}

fn delete_requests() {
    let encoded = encode_video_id_with_provider("vid-5", "openai", Some(MODEL));
    let (url, params) = delete_request(&encoded, "https://mock.example/v1/videos/").unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid-5");
    assert!(params.is_empty());

    let missing = delete_request("", API_BASE).unwrap_err();
    assert!(matches!(
        missing,
        VideoError::Path { source: HttpError::Required { ref field_name } } if field_name == "video_id"
    ));
    assert_eq!(missing.to_string(), "video_id is required");
    let dot = delete_request(".", API_BASE).unwrap_err();
    assert!(matches!(
        dot,
        VideoError::Path { source: HttpError::DotSegment { ref field_name } } if field_name == "video_id"
    ));
    assert_eq!(dot.to_string(), "video_id cannot be a dot path segment");
    let dot_dot = delete_request("..", API_BASE).unwrap_err();
    assert!(matches!(
        dot_dot,
        VideoError::Path {
            source: HttpError::DotSegment { .. }
        }
    ));
}

fn delete_reply() {
    let reply = json!({
        "id": "vid-5",
        "object": "video",
        "status": "deleted",
        "seconds": "4",
        "usage": {"duration_seconds": 99}
    });
    let deleted = delete_response(&body(&reply)).unwrap();
    assert_eq!(deleted.id, "vid-5");
    assert_eq!(deleted.seconds, Some(json!("4")));
    assert_eq!(
        Value::Object(deleted.usage.unwrap()),
        json!({"duration_seconds": 99})
    );
    assert!(matches!(
        delete_response(&body(&json!({"data": []}))),
        Err(VideoError::ExpectedVideo)
    ));
}

fn error_classes() {
    let error = get_error_class("rate limited", 429, headers(&[("retry-after", "1")]));
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rate limited");
    assert_eq!(
        error.headers,
        Some(vec![("retry-after".to_owned(), "1".to_owned())])
    );
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.request.method, "POST");
    assert_eq!(error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.response.status_code, 429);
    assert_eq!(error.body, None);
    let empty = get_error_class("missing", 400, headers(&[]));
    assert_eq!(empty.headers, Some(Vec::new()));
}

fn delete_case() {
    delete_requests();
    delete_reply();
    error_classes();
}

fn binary_content_case() {
    let bytes: Vec<u8> = (0..=255).collect();
    assert_eq!(content_response(&bytes), bytes);
    assert!(content_response(&[]).is_empty());

    let encoded = encode_video_id_with_provider("vid-6", "openai", Some(MODEL));
    let (url, params) = content_request(&encoded, "https://mock.example/v1/videos/", None).unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid-6/content");
    assert!(params.is_empty());
}

fn invalid_seconds_case() {
    for text in [
        "abc",
        "",
        "  ",
        "1,5",
        "0x10",
        "nan",
        "inf",
        "-infinity",
        "1_0",
        "٤",
    ] {
        let video = seconds_video(Some(json!(text)));
        assert_eq!(video.seconds, Some(json!(text)));
        assert!(video.usage.unwrap().is_empty());
    }

    let truthy_boolean = seconds_video(Some(json!(true)));
    assert_duration(&truthy_boolean, "1.0");
    let false_boolean = seconds_video(Some(json!(false)));
    assert!(false_boolean.usage.unwrap().is_empty());
    let absent = seconds_video(None);
    assert_eq!(absent.seconds, None);
    assert!(absent.usage.unwrap().is_empty());
    let null = seconds_video(Some(Value::Null));
    assert_eq!(null.seconds, Some(Value::Null));
    assert!(null.usage.unwrap().is_empty());
    let array = seconds_video(Some(json!(["4"])));
    assert!(array.usage.unwrap().is_empty());

    let remixed = remix_response(
        &body(&json!({"id": "vid-7", "object": "video", "status": "queued", "seconds": "abc"})),
        None,
    )
    .unwrap();
    assert_eq!(remixed.seconds, Some(json!("abc")));
    assert!(remixed.usage.unwrap().is_empty());
}

fn assert_duration(video: &VideoObject, expected: &str) {
    let usage = video.usage.as_ref().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        serde_json::to_string(usage.get("duration_seconds").unwrap()).unwrap(),
        expected
    );
}

fn create_durations() {
    let reply = json!({
        "id": "vid-8",
        "object": "video",
        "status": "completed",
        "seconds": "4",
        "usage": {"duration_seconds": 99, "other": true}
    });
    let created = create_response(MODEL, &body(&reply), Some("openai")).unwrap();
    assert_eq!(video_of(&created.id), "vid-8");
    assert_eq!(model_of(&created.id).as_deref(), Some(MODEL));
    assert_duration(&created, "4.0");

    let zero = create_response(
        MODEL,
        &body(&json!({"id": "vid-8", "object": "video", "status": "completed", "seconds": "0"})),
        Some("openai"),
    )
    .unwrap();
    assert_duration(&zero, "0.0");

    let decimal = create_response(
        MODEL,
        &body(&json!({"id": "vid-8", "object": "video", "status": "completed", "seconds": "2.5"})),
        None,
    )
    .unwrap();
    assert_eq!(decimal.id, "vid-8");
    assert_duration(&decimal, "2.5");

    let numeric_zero = create_response(
        MODEL,
        &body(&json!({"id": "vid-8", "object": "video", "status": "completed", "seconds": 0})),
        None,
    )
    .unwrap();
    assert!(numeric_zero.usage.unwrap().is_empty());
    let numeric = create_response(
        MODEL,
        &body(&json!({"id": "vid-8", "object": "video", "status": "completed", "seconds": 4})),
        None,
    )
    .unwrap();
    assert_duration(&numeric, "4.0");

    let empty_id = create_response(
        MODEL,
        &body(&json!({"id": "", "object": "video", "status": "completed", "seconds": "3"})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(empty_id.id, "");
}

fn duration_case() {
    create_durations();
    let status = status_response(
        &body(&json!({"id": "vid-8", "object": "video", "status": "completed", "seconds": "4"})),
        Some("openai"),
    )
    .unwrap();
    assert_eq!(status.usage, None);
    let remixed = remix_response(
        &body(&json!({"id": "vid-8", "object": "video", "status": "queued", "seconds": "4"})),
        Some("openai"),
    )
    .unwrap();
    assert_duration(&remixed, "4.0");
}

fn bearer(configured: &VideoCredentials) -> String {
    let base = secret_headers(&[("Authorization", "Bearer old"), ("X-Trace", "trace-1")]);
    let updated = validate_environment(base, configured);
    assert_eq!(exposed(&updated, "X-Trace"), Some("trace-1"));
    exposed(&updated, "Authorization")
        .unwrap_or_default()
        .to_owned()
}

fn auth_chain_case() {
    assert_eq!(
        bearer(&credentials(
            Some("explicit"),
            Some("params"),
            Some("global"),
            Some("openai"),
            Some("environment"),
        )),
        "Bearer explicit"
    );
    assert_eq!(
        bearer(&credentials(
            None,
            Some("params"),
            Some("global"),
            Some("openai"),
            Some("environment"),
        )),
        "Bearer params"
    );
    assert_eq!(
        bearer(&credentials(
            Some(""),
            Some("params"),
            Some("global"),
            Some("openai"),
            Some("environment"),
        )),
        "Bearer params"
    );
    assert_eq!(
        bearer(&credentials(
            None,
            None,
            Some("global"),
            Some("openai"),
            Some("environment")
        )),
        "Bearer global"
    );
    assert_eq!(
        bearer(&credentials(
            None,
            None,
            None,
            Some("openai"),
            Some("environment")
        )),
        "Bearer openai"
    );
    assert_eq!(
        bearer(&credentials(None, None, None, None, Some("environment"))),
        "Bearer environment"
    );
    assert_eq!(
        bearer(&credentials(
            Some(""),
            Some(""),
            Some(""),
            Some(""),
            Some("")
        )),
        "Bearer None"
    );
    assert_eq!(
        bearer(&credentials(Some(""), None, None, None, None)),
        "Bearer None"
    );
    assert_eq!(
        bearer(&credentials(Some(" "), None, None, None, None)),
        "Bearer  "
    );
    assert_eq!(complete_url(None), format!("{DEFAULT_API_BASE}/videos"));
    assert_eq!(
        complete_url(Some("https://mock.example/v1///")),
        "https://mock.example/v1/videos"
    );
    assert_eq!(complete_url(Some("")), "/videos");
}

fn content_variant_case() {
    let encoded = encode_video_id_with_provider("vid/10", "openai", Some(MODEL));
    let (url, params) = content_request(&encoded, "https://mock.example/v1/videos/", None).unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid%2F10/content");
    assert!(params.is_empty());

    let (url, _) = content_request(&encoded, "https://mock.example/v1/videos", Some("")).unwrap();
    assert_eq!(
        url,
        "https://mock.example/v1/videos/vid%2F10/content?variant="
    );
    let (url, _) = content_request(
        &encoded,
        "https://mock.example/v1/videos",
        Some("thumbnail&extra=1"),
    )
    .unwrap();
    assert_eq!(
        url,
        "https://mock.example/v1/videos/vid%2F10/content?variant=thumbnail%26extra%3D1"
    );
    let (url, _) =
        content_request(&encoded, "https://mock.example/v1/videos", Some("héllo")).unwrap();
    assert_eq!(
        url,
        "https://mock.example/v1/videos/vid%2F10/content?variant=h%C3%A9llo"
    );

    assert!(matches!(
        content_request("", API_BASE, None),
        Err(VideoError::Path {
            source: HttpError::Required { .. }
        })
    ));
    assert!(matches!(
        content_request("..", API_BASE, None),
        Err(VideoError::Path {
            source: HttpError::DotSegment { .. }
        })
    ));
}

#[rstest]
#[case::video_create_multipart(0)]
#[case::input_reference(1)]
#[case::list_and_status(2)]
#[case::remix_request(3)]
#[case::delete_response(4)]
#[case::binary_content(5)]
#[case::invalid_seconds_kept_without_usage(6)]
#[case::duration_seconds_decimal(7)]
#[case::video_auth_chain(8)]
#[case::video_content_variant(9)]
fn videos_cases(#[case] case: u8) {
    match case {
        0 => create_multipart_case(),
        1 => input_reference_case(),
        2 => list_and_status_case(),
        3 => remix_case(),
        4 => delete_case(),
        5 => binary_content_case(),
        6 => invalid_seconds_case(),
        7 => duration_case(),
        8 => auth_chain_case(),
        _ => content_variant_case(),
    }
}
