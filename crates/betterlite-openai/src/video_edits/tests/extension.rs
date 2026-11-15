use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::video_edits::extension::{VideoExtensionRequest, transform_video_extension_request};
use crate::video_edits::response::{VideoKind, VideoReply, transform_video_response};

use super::support::{encoded_video, json, keys, options};

const VIDEO: &str = r#"{"id":"video_1","object":"video","status":"completed","created_at":1700000000,"error":null,"progress":null,"seconds":"8","size":"1280x720","model":null,"usage":null}"#;

fn extension_request(prompt: &str, video_id: &str) -> VideoExtensionRequest {
    VideoExtensionRequest {
        prompt: prompt.to_owned(),
        video_id: video_id.to_owned(),
        seconds: String::from("8"),
        api_base: String::from("https://mock.example/v1/"),
        extra_body: None,
    }
}

pub(super) fn video_extension() {
    let encoded = encoded_video("vid-123");
    let extension = transform_video_extension_request(extension_request("extend", &encoded));
    assert_eq!(extension.url, "https://mock.example/v1/extensions");
    assert_eq!(keys(&extension.body), ["prompt", "seconds", "video"]);
    assert_eq!(extension.body.get("prompt"), Some(&json(r#""extend""#)));
    assert_eq!(extension.body.get("seconds"), Some(&json(r#""8""#)));
    assert_eq!(
        extension.body.get("video"),
        Some(&json(r#"{"id":"vid-123"}"#))
    );

    let mut request = extension_request("extend", &encoded);
    request.seconds = String::from("08");
    let supplied = transform_video_extension_request(request);
    assert_eq!(supplied.body.get("seconds"), Some(&json(r#""08""#)));

    let mut request = extension_request("extend", &encoded);
    request.extra_body = Some(options(
        r#"{"prompt":"again","seconds":"12","video":{"id":"other"}}"#,
    ));
    let overwritten = transform_video_extension_request(request);
    assert_eq!(keys(&overwritten.body), ["prompt", "seconds", "video"]);
    assert_eq!(overwritten.body.get("prompt"), Some(&json(r#""again""#)));
    assert_eq!(overwritten.body.get("seconds"), Some(&json(r#""12""#)));
    assert_eq!(
        overwritten.body.get("video"),
        Some(&json(r#"{"id":"other"}"#))
    );

    let reply = VideoReply {
        body: VIDEO.as_bytes(),
        custom_llm_provider: Some("azure"),
        request_data: None,
    };
    let wrapped = transform_video_response(&reply).expect("a video reply");
    assert_eq!(
        wrapped.id,
        encode_video_id_with_provider("video_1", "azure", None)
    );
    assert_eq!(wrapped.object, VideoKind::Video);
    assert_eq!(wrapped.status, "completed");
    assert_eq!(wrapped.created_at, Some(1_700_000_000));
    assert_eq!(wrapped.seconds.as_deref(), Some("8"));
    assert_eq!(wrapped.size.as_deref(), Some("1280x720"));
    assert!(wrapped.model.is_none(), "the wrapped id keeps model unset");
    assert!(wrapped.usage.is_none(), "no usage is invented");
    assert!(wrapped.completed_at.is_none());
    assert!(wrapped.expires_at.is_none());
    assert!(wrapped.error.is_none());
    assert!(wrapped.progress.is_none());
    assert!(wrapped.remixed_from_video_id.is_none());

    let missing = VideoReply {
        body: VIDEO.as_bytes(),
        custom_llm_provider: None,
        request_data: None,
    };
    let unwrapped = transform_video_response(&missing).expect("a video reply");
    assert_eq!(unwrapped.id, "video_1");
    let empty_provider = VideoReply {
        body: VIDEO.as_bytes(),
        custom_llm_provider: Some(""),
        request_data: None,
    };
    let unwrapped = transform_video_response(&empty_provider).expect("a video reply");
    assert_eq!(unwrapped.id, "video_1");
    let empty_id = VideoReply {
        body: br#"{"id":"","object":"video","status":"completed"}"#,
        custom_llm_provider: Some("openai"),
        request_data: None,
    };
    let unwrapped = transform_video_response(&empty_id).expect("a video reply");
    assert_eq!(unwrapped.id, "");

    let wrong_object = VideoReply {
        body: br#"{"id":"v1","object":"clip","status":"completed"}"#,
        custom_llm_provider: None,
        request_data: None,
    };
    assert!(transform_video_response(&wrong_object).is_err());
    let missing_status = VideoReply {
        body: br#"{"id":"v1","object":"video"}"#,
        custom_llm_provider: None,
        request_data: None,
    };
    assert!(transform_video_response(&missing_status).is_err());
    let not_json = VideoReply {
        body: b"{",
        custom_llm_provider: None,
        request_data: None,
    };
    assert!(transform_video_response(&not_json).is_err());
}
