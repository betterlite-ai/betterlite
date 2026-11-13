use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;
use serde_json::{Value, json};

use super::fixtures::{Capture, Failure, GIF, HEIC, JPEG, PNG, ProbeReader, WEBP};
use crate::openai_video_identifiers::ids::{
    decode_character_id, decode_video_id_with_provider, encode_character_id,
    encode_video_id_with_provider, rewrite_character_ids,
};
use crate::openai_video_identifiers::multipart::{
    MediaBody, MultipartPart, add_image_to_files, video_file_tuple,
};

#[rstest]
#[case::video_id_roundtrip(0)]
#[case::character_id_roundtrip(1)]
#[case::invalid_id_fallback(2)]
#[case::input_reference_string_png(3)]
#[case::multipart_input_file(4)]
#[case::encode_identity_guards(5)]
#[case::decode_permissive_fields(6)]
#[case::character_list_rewrite(7)]
#[case::video_filename_mime(8)]
#[case::video_probe_cursor(9)]
fn openai_video_identifiers_cases(#[case] case: u8) {
    match case {
        0 => video_id_roundtrip(),
        1 => character_id_roundtrip(),
        2 => invalid_id_fallback(),
        3 => input_reference_string_png(),
        4 => multipart_input_file(),
        5 => encode_identity_guards(),
        6 => decode_permissive_fields(),
        7 => character_list_rewrite(),
        8 => video_filename_mime(),
        _ => video_probe_cursor(),
    }
}

fn video_id_roundtrip() {
    let payload = "litellm:custom_llm_provider:openai;model_id:sora-2;video_id:vid-123";
    let encoded = encode_video_id_with_provider("vid-123", "openai", Some("sora-2"));
    assert_eq!(encoded, format!("video_{}", STANDARD.encode(payload)));
    let roundtrip = decode_video_id_with_provider(&encoded);
    assert_eq!(roundtrip.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(roundtrip.model_id.as_deref(), Some("sora-2"));
    assert_eq!(roundtrip.video_id, "vid-123");
    let empty_model = encode_video_id_with_provider("vid-456", "azure", None);
    let expected = "litellm:custom_llm_provider:azure;model_id:;video_id:vid-456";
    assert_eq!(empty_model, format!("video_{}", STANDARD.encode(expected)));
    let azure_roundtrip = decode_video_id_with_provider(&empty_model);
    assert_eq!(
        azure_roundtrip.custom_llm_provider.as_deref(),
        Some("azure")
    );
    assert_eq!(azure_roundtrip.model_id.as_deref(), Some(""));
    assert_eq!(azure_roundtrip.video_id, "vid-456");
}

fn character_id_roundtrip() {
    let payload = "litellm:custom_llm_provider:openai;model_id:sora-2;character_id:char-1";
    let encoded = encode_character_id("char-1", "openai", Some("sora-2"));
    assert_eq!(encoded, format!("character_{}", STANDARD.encode(payload)));
    let decoded = decode_character_id(&encoded);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(decoded.model_id.as_deref(), Some("sora-2"));
    assert_eq!(decoded.character_id, "char-1");
    let empty_model = encode_character_id("char-2", "azure", Some(""));
    let expected = "litellm:custom_llm_provider:azure;model_id:;character_id:char-2";
    assert_eq!(
        empty_model,
        format!("character_{}", STANDARD.encode(expected))
    );
    assert_eq!(
        decode_character_id(&empty_model).model_id.as_deref(),
        Some("")
    );
}

const UNPREFIXED_VIDEO: &str =
    "vid_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTt2aWRlb19pZDp2";
const UNPREFIXED_CHARACTER: &str =
    "char_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtjaGFyYWN0ZXJfaWQ6Yw==";

fn invalid_id_fallback() {
    let two_fields = format!("video_{}", STANDARD.encode("a;b"));
    let video_cases = [
        ("", 0),
        ("vid_abc", 0),
        (UNPREFIXED_VIDEO, 0),
        (two_fields.as_str(), 0),
        ("video_YQ", 0),
        ("video_YWI", 0),
        ("video_YQ---", 0),
        ("video_AAAAA", 1),
        ("video_YTtiO2M-", 1),
        ("video_/w==", 1),
    ];
    for (encoded, debug_events) in video_cases {
        let capture = Capture::default();
        let decoded = capture.captured(|| decode_video_id_with_provider(encoded));
        assert_eq!(decoded.video_id, encoded);
        assert!(decoded.custom_llm_provider.is_none(), "{encoded}");
        assert!(decoded.model_id.is_none(), "{encoded}");
        assert_eq!(capture.events().len(), debug_events, "{encoded}");
    }
    let unseparated = format!("character_{}", STANDARD.encode("plain"));
    let character_cases = [
        ("", 0),
        (UNPREFIXED_CHARACTER, 0),
        (unseparated.as_str(), 0),
        ("character_AAAAA", 1),
        ("character_/w==", 1),
    ];
    for (encoded, debug_events) in character_cases {
        let capture = Capture::default();
        let decoded = capture.captured(|| decode_character_id(encoded));
        assert_eq!(decoded.character_id, encoded);
        assert!(decoded.custom_llm_provider.is_none(), "{encoded}");
        assert!(decoded.model_id.is_none(), "{encoded}");
        assert_eq!(capture.events().len(), debug_events, "{encoded}");
    }
}

fn input_reference_string_png() {
    let mut files = Vec::new();
    add_image_to_files(
        &mut files,
        "input_reference",
        MediaBody::Text("https://example.invalid/a.png"),
    );
    let part = files.first().expect("image part");
    assert_eq!(part.field, "input_reference");
    assert_eq!(part.filename, "input_reference.png");
    assert_eq!(part.content_type, "image/png");
    assert!(
        matches!(&part.body, MediaBody::Text(text) if *text == "https://example.invalid/a.png")
    );
}

fn added_image(body: MediaBody<'_>) -> (String, &'static str) {
    let mut files = Vec::new();
    add_image_to_files(&mut files, "input_reference", body);
    let part = files.first().expect("image part");
    (part.filename.clone(), part.content_type)
}

fn earlier_field_is_kept() {
    let mut reader = ProbeReader::new(PNG, 3, None);
    let mut files = vec![MultipartPart {
        field: "prompt".to_owned(),
        filename: String::new(),
        content_type: "text/plain",
        body: MediaBody::Text("keep"),
    }];
    add_image_to_files(
        &mut files,
        "input_reference",
        MediaBody::Stream {
            name: Some("ref.png"),
            reader: &mut reader,
        },
    );
    assert_eq!(files.len(), 2);
    assert_eq!(files.first().expect("earlier field").field, "prompt");
    let (filename, content_type) = {
        let part = files.last().expect("image part");
        (part.filename.clone(), part.content_type)
    };
    drop(files);
    assert_eq!(filename, "ref.png");
    assert_eq!(content_type, "image/png");
    assert_eq!(reader.position(), 3);
}

fn multipart_input_file() {
    earlier_field_is_kept();
    let mut unnamed = ProbeReader::new(JPEG, 0, None);
    let jpeg_stream = MediaBody::Stream {
        name: None,
        reader: &mut unnamed,
    };
    assert_eq!(
        added_image(jpeg_stream),
        ("input_reference.png".to_owned(), "image/jpeg")
    );
    assert_eq!(
        added_image(MediaBody::Bytes(JPEG)),
        ("input_reference.png".to_owned(), "image/jpeg")
    );
    for (bytes, expected) in [
        (GIF, "image/gif"),
        (WEBP, "image/webp"),
        (HEIC, "image/heic"),
    ] {
        assert_eq!(added_image(MediaBody::Bytes(bytes)).1, expected);
    }
}

fn encode_identity_guards() {
    assert_eq!(encode_video_id_with_provider("vid", "", None), "vid");
    assert_eq!(encode_video_id_with_provider("", "openai", None), "");
    assert_eq!(encode_character_id("char", "", None), "char");
    assert_eq!(encode_character_id("", "openai", None), "");
    let video = encode_video_id_with_provider("vid", "openai", Some("sora-2"));
    assert_eq!(
        encode_video_id_with_provider(&video, "azure", Some("other")),
        video
    );
    let character = encode_character_id("char", "openai", Some("sora-2"));
    assert_eq!(
        encode_character_id(&character, "azure", Some("other")),
        character
    );
    let empty_provider = format!(
        "video_{}",
        STANDARD.encode("litellm:custom_llm_provider:;model_id:;video_id:vid")
    );
    assert_eq!(
        encode_video_id_with_provider(&empty_provider, "openai", None),
        empty_provider
    );
    let empty_character_provider = format!(
        "character_{}",
        STANDARD.encode("litellm:custom_llm_provider:;model_id:;character_id:char")
    );
    assert_eq!(
        encode_character_id(&empty_character_provider, "openai", None),
        empty_character_provider
    );
}

fn decode_permissive_fields() {
    let payload = "litellm:custom_llm_provider:openai;model_id:sora;video_id:vid";
    let padded = STANDARD.encode(payload);
    assert!(padded.ends_with('='));
    let unpadded = padded.trim_end_matches('=');
    let decoded = decode_video_id_with_provider(&format!("video_{unpadded}"));
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(decoded.video_id, "vid");
    let doubled = decode_video_id_with_provider(&format!("video_video_{padded}"));
    assert_eq!(doubled.video_id, "vid");
    let bare = decode_video_id_with_provider(&format!("video_{}", STANDARD.encode("x;y;z")));
    assert_eq!(bare.custom_llm_provider.as_deref(), Some("x"));
    assert_eq!(bare.model_id.as_deref(), Some("y"));
    assert_eq!(bare.video_id, "z");
    let embedded =
        "pre litellm:custom_llm_provider:mid;mo del model_id: del;video_id:one video_id:two";
    let embedded_decoded =
        decode_video_id_with_provider(&format!("video_{}", STANDARD.encode(embedded)));
    assert_eq!(
        embedded_decoded.custom_llm_provider.as_deref(),
        Some("pre mid")
    );
    assert_eq!(embedded_decoded.model_id.as_deref(), Some("mo del del"));
    assert_eq!(embedded_decoded.video_id, "one two");
    let extra = decode_video_id_with_provider(&format!("video_{}", STANDARD.encode("a;b;c;d;e")));
    assert_eq!(extra.video_id, "c");
    let character = decode_character_id(&format!(
        "character_{}",
        STANDARD.encode(
            "litellm:custom_llm_provider:p;model_id:m;character_id:one character_id:two;extra"
        )
    ));
    assert_eq!(character.custom_llm_provider.as_deref(), Some("p"));
    assert_eq!(character.character_id, "one two");
}

fn character_list_rewrite() {
    let encoded = encode_character_id("char-1", "openai", Some("sora-2"));
    let mut request = json!({
        "model": "sora-2",
        "prompt": "make a video",
        "seconds": "4",
        "characters": [
            {"id": encoded, "role": "hero"},
            {"role": "plain"},
            {"id": 7},
            "raw",
            3
        ]
    })
    .as_object()
    .cloned()
    .expect("object");
    rewrite_character_ids(&mut request);
    let expected = json!({
        "model": "sora-2",
        "prompt": "make a video",
        "seconds": "4",
        "characters": [{"id": "char-1", "role": "hero"}, {"role": "plain"}, {"id": 7}, "raw", 3]
    });
    assert_eq!(Value::Object(request), expected);
    let mut plain = json!({"model": "m", "characters": "none"})
        .as_object()
        .cloned()
        .expect("object");
    rewrite_character_ids(&mut plain);
    assert_eq!(plain.get("characters"), Some(&json!("none")));
}

fn streamed_video(
    name: Option<&str>,
    data: &[u8],
    position: usize,
    failure: Option<Failure>,
) -> (String, &'static str, usize) {
    let mut reader = ProbeReader::new(data, position, failure);
    let (filename, content_type) = {
        let part = video_file_tuple(
            MediaBody::Stream {
                name,
                reader: &mut reader,
            },
            "video",
        );
        (part.filename.clone(), part.content_type)
    };
    (filename, content_type, reader.position())
}

fn ftyp_after_ten_bytes() -> Vec<u8> {
    let mut ftyp = vec![0_u8; 80];
    ftyp.get_mut(10..14)
        .expect("range")
        .copy_from_slice(b"ftyp");
    ftyp
}

fn video_filename_mime() {
    assert_eq!(
        streamed_video(Some("clip.webm"), b"no signature", 0, None),
        ("clip.webm".to_owned(), "video/webm", 0)
    );
    let ftyp = ftyp_after_ten_bytes();
    assert_eq!(
        streamed_video(Some("clip.png"), &ftyp, 0, None).1,
        "video/mp4"
    );
    assert_eq!(
        streamed_video(Some("clip.png"), b"no ftyp", 0, None).1,
        "video/mp4"
    );
    let named_empty = streamed_video(Some(""), b"", 0, None);
    assert_eq!(
        (named_empty.0.as_str(), named_empty.1),
        ("input_video.mp4", "video/mp4")
    );
    for body in [MediaBody::Bytes(b"no name"), MediaBody::Text("a string")] {
        let part = video_file_tuple(body, "video");
        assert_eq!(part.field, "video");
        assert_eq!(part.filename, "input_video.mp4");
        assert_eq!(part.content_type, "video/mp4");
    }
}

fn video_probe_cursor() {
    let ftyp = ftyp_after_ten_bytes();
    let cases = [
        (3, None, 3),
        (2, None, 2),
        (3, Some(Failure::Tell), 3),
        (3, Some(Failure::Seek), 3),
        (3, Some(Failure::Read), 0),
        (3, Some(Failure::Restore), 64),
    ];
    for (position, failure, after) in cases {
        let (_, content_type, cursor) = streamed_video(Some("clip.bin"), &ftyp, position, failure);
        assert_eq!(content_type, "video/mp4", "{failure:?}");
        assert_eq!(cursor, after, "{failure:?} from {position}");
    }
}
