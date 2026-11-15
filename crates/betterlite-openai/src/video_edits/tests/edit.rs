use std::io::Cursor;

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::video_edits::edit::{VideoEditRequest, transform_video_edit_request};
use crate::video_edits::response::{VideoReply, transform_video_response};

use super::support::{encoded_video, json, keys, options};

fn text_request<'media>(prompt: &str, video_id: &str) -> VideoEditRequest<'media> {
    VideoEditRequest {
        prompt: prompt.to_owned(),
        video_id: video_id.to_owned(),
        api_base: String::from("https://mock.example/v1"),
        video_file: None,
        extra_body: None,
        prefetched_source_data: None,
    }
}

pub(super) fn edit_prompt() {
    json_body();
    supplied_file();
}

fn json_body() {
    let encoded = encoded_video("vid-123");
    let mut request = text_request("make it blue", &encoded);
    request.extra_body = Some(options(r#"{"prompt":"override","duration":8}"#));
    let edit = transform_video_edit_request(request);
    assert_eq!(edit.url, "https://mock.example/v1/edits");
    assert_eq!(keys(&edit.body), ["prompt", "video", "duration"]);
    assert_eq!(edit.body.get("prompt"), Some(&json(r#""override""#)));
    assert_eq!(edit.body.get("video"), Some(&json(r#"{"id":"vid-123"}"#)));
    assert_eq!(edit.body.get("duration"), Some(&json("8")));
    assert!(edit.files.is_none());

    let plain = transform_video_edit_request(text_request("make it blue", &encoded));
    assert_eq!(keys(&plain.body), ["prompt", "video"]);
    assert_eq!(plain.body.get("prompt"), Some(&json(r#""make it blue""#)));

    let mut request = text_request("make it blue", &encoded);
    request.api_base = String::from("https://mock.example/v1/");
    let slashed = transform_video_edit_request(request);
    assert_eq!(slashed.url, "https://mock.example/v1/edits");
}

fn supplied_file() {
    let encoded = encoded_video("vid-123");
    let mut cursor = Cursor::new(b"not video".to_vec());
    let mut request = text_request("make it blue", &encoded);
    request.video_file = Some(MediaBody::Stream {
        name: Some("clip.webm"),
        reader: &mut cursor,
    });
    request.extra_body = Some(options(r#"{"prompt":"override","duration":8}"#));
    let edit = transform_video_edit_request(request);
    assert_eq!(edit.url, "https://mock.example/v1/edits");
    assert_eq!(
        keys(&edit.body),
        ["prompt", "duration"],
        "the file wins over the id"
    );
    assert_eq!(edit.body.get("prompt"), Some(&json(r#""override""#)));
    let Some(video) = edit.files else {
        panic!("a supplied file selects the multipart form");
    };
    assert_eq!(video.field, "video");
    assert_eq!(video.filename, "clip.webm");
    assert_eq!(video.content_type, "video/webm");

    let mut request = text_request("make it blue", &encoded);
    request.video_file = Some(MediaBody::Bytes(b""));
    let empty_file = transform_video_edit_request(request);
    assert!(empty_file.files.is_some(), "an empty file is still a file");
    assert_eq!(keys(&empty_file.body), ["prompt"]);
}

pub(super) fn unused_prefetched_source() {
    let encoded = encoded_video("vid-9");
    let mut request = text_request("extend it", &encoded);
    request.prefetched_source_data = Some(json(
        r#"{"video":{"id":"prefetched"},"url":"https://prefetched.invalid/v.mp4"}"#,
    ));
    request.extra_body = Some(options(r#"{"duration":4}"#));
    let edit = transform_video_edit_request(request);
    assert_eq!(keys(&edit.body), ["prompt", "video", "duration"]);
    assert_eq!(edit.body.get("video"), Some(&json(r#"{"id":"vid-9"}"#)));
    assert!(edit.files.is_none());

    let body = br#"{"id":"video_7","object":"video","status":"completed"}"#;
    let request_data = json(r#"{"prompt":"extend it","video":{"id":"vid-9"}}"#);
    let reply = VideoReply {
        body,
        custom_llm_provider: Some("openai"),
        request_data: Some(&request_data),
    };
    let with_data = transform_video_response(&reply).expect("a video reply");
    let reply = VideoReply {
        body,
        custom_llm_provider: Some("openai"),
        request_data: None,
    };
    let without_data = transform_video_response(&reply).expect("a video reply");
    assert_eq!(with_data, without_data);
    assert_eq!(
        with_data.id,
        encode_video_id_with_provider("video_7", "openai", None)
    );
}
