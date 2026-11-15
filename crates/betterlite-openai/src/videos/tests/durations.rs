use serde_json::{Value, json};

use crate::videos::create::create_response;
use crate::videos::remix::remix_response;
use crate::videos::reply::VideoObject;
use crate::videos::status::status_response;

use super::support::{MODEL, model_of, video_body, video_of};

const INVALID_TEXTS: [&str; 10] = [
    "abc",
    "",
    "  ",
    "1,5",
    "0x10",
    "nan",
    "inf",
    "-infinity",
    "1_0",
    "\u{664}",
];

fn seconds_video(seconds: Option<Value>) -> VideoObject {
    create_response(MODEL, &video_body("vid-7", seconds), Some("openai")).unwrap()
}

fn assert_duration(video: &VideoObject, expected: &str) {
    let usage = video.usage.as_ref().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        serde_json::to_string(usage.get("duration_seconds").unwrap()).unwrap(),
        expected
    );
}

fn assert_no_duration(video: &VideoObject) {
    assert!(video.usage.as_ref().unwrap().is_empty());
}

fn invalid_text_case() {
    for text in INVALID_TEXTS {
        let video = seconds_video(Some(json!(text)));
        assert_eq!(video.seconds, Some(json!(text)));
        assert_no_duration(&video);
    }
}

fn invalid_type_case() {
    assert_duration(&seconds_video(Some(json!(true))), "1.0");
    assert_no_duration(&seconds_video(Some(json!(false))));
    assert_no_duration(&seconds_video(Some(json!(["4"]))));
    let absent = seconds_video(None);
    assert_eq!(absent.seconds, None);
    assert_no_duration(&absent);
    let null = seconds_video(Some(Value::Null));
    assert_eq!(null.seconds, Some(Value::Null));
    assert_no_duration(&null);
}

pub(super) fn invalid_seconds_case() {
    invalid_text_case();
    invalid_type_case();
    let remixed = remix_response(&video_body("vid-7", Some(json!("abc"))), None).unwrap();
    assert_eq!(remixed.seconds, Some(json!("abc")));
    assert_no_duration(&remixed);
}

fn create_wrapped_case() {
    let reply = json!({
        "id": "vid-8",
        "object": "video",
        "status": "completed",
        "seconds": "4",
        "usage": {"duration_seconds": 99_i64, "other": true}
    });
    let bytes = serde_json::to_vec(&reply).unwrap();
    let created = create_response(MODEL, &bytes, Some("openai")).unwrap();
    assert_eq!(video_of(&created.id), "vid-8");
    assert_eq!(model_of(&created.id).as_deref(), Some(MODEL));
    assert_duration(&created, "4.0");
}

fn create_seconds_forms_case() {
    let texts = [("0", "0.0"), ("2.5", "2.5"), ("3", "3.0")];
    for (text, expected) in texts {
        let reply = video_body("vid-8", Some(json!(text)));
        assert_duration(&create_response(MODEL, &reply, None).unwrap(), expected);
    }
    let numeric = video_body("vid-8", Some(json!(4_i64)));
    assert_duration(&create_response(MODEL, &numeric, None).unwrap(), "4.0");
    let zero = video_body("vid-8", Some(json!(0_i64)));
    assert_no_duration(&create_response(MODEL, &zero, None).unwrap());
}

fn create_native_case() {
    let decimal = create_response(MODEL, &video_body("vid-8", Some(json!("2.5"))), None).unwrap();
    assert_eq!(decimal.id, "vid-8");
    let empty_id = create_response(MODEL, &video_body("", Some(json!("3"))), Some("openai"));
    assert_eq!(empty_id.unwrap().id, "");
}

pub(super) fn duration_case() {
    create_wrapped_case();
    create_seconds_forms_case();
    create_native_case();
    let seconds = Some(json!("4"));
    let status = status_response(&video_body("vid-8", seconds.clone()), Some("openai")).unwrap();
    assert_eq!(status.usage, None);
    let remixed = remix_response(&video_body("vid-8", seconds), Some("openai")).unwrap();
    assert_duration(&remixed, "4.0");
}
