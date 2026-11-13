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

fn invalid_id_fallback() {
    let empty_capture = Capture::default();
    let empty = empty_capture.captured(|| decode_video_id_with_provider(""));
    assert_eq!(empty.video_id, "");
    assert!(empty.custom_llm_provider.is_none());
    assert!(empty_capture.events().is_empty());
    let prefix_capture = Capture::default();
    let wrong_prefix = prefix_capture.captured(|| decode_video_id_with_provider("vid_abc"));
    assert_eq!(wrong_prefix.video_id, "vid_abc");
    assert!(prefix_capture.events().is_empty());
    let separator_capture = Capture::default();
    let unseparated = separator_capture
        .captured(|| decode_character_id(&format!("character_{}", STANDARD.encode("plain"))));
    assert_eq!(
        unseparated.character_id,
        format!("character_{}", STANDARD.encode("plain"))
    );
    assert!(unseparated.custom_llm_provider.is_none());
    assert!(separator_capture.events().is_empty());
    let short_capture = Capture::default();
    let two_fields = short_capture
        .captured(|| decode_video_id_with_provider(&format!("video_{}", STANDARD.encode("a;b"))));
    assert_eq!(
        two_fields.video_id,
        format!("video_{}", STANDARD.encode("a;b"))
    );
    assert!(two_fields.model_id.is_none());
    assert!(short_capture.events().is_empty());
    let base64_capture = Capture::default();
    let bad_base64 = base64_capture.captured(|| decode_video_id_with_provider("video_AAAAA"));
    assert_eq!(bad_base64.video_id, "video_AAAAA");
    assert_eq!(base64_capture.events().len(), 1);
    let utf8_capture = Capture::default();
    let bad_utf8 = utf8_capture.captured(|| decode_video_id_with_provider("video_/w=="));
    assert_eq!(bad_utf8.video_id, "video_/w==");
    assert_eq!(utf8_capture.events().len(), 1);
    let character_capture = Capture::default();
    let character = character_capture.captured(|| decode_character_id("character_/w=="));
    assert_eq!(character.character_id, "character_/w==");
    assert_eq!(character_capture.events().len(), 1);
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

fn multipart_input_file() {
    {
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
    {
        let mut reader = ProbeReader::new(JPEG, 0, None);
        let mut files = Vec::new();
        add_image_to_files(
            &mut files,
            "input_reference",
            MediaBody::Stream {
                name: None,
                reader: &mut reader,
            },
        );
        let part = files.first().expect("image part");
        assert_eq!(part.filename, "input_reference.png");
        assert_eq!(part.content_type, "image/jpeg");
        assert!(matches!(&part.body, MediaBody::Stream { name: None, .. }));
    }
    {
        let mut files = Vec::new();
        add_image_to_files(&mut files, "input_reference", MediaBody::Bytes(JPEG));
        let part = files.first().expect("image part");
        assert_eq!(part.filename, "input_reference.png");
        assert_eq!(part.content_type, "image/jpeg");
        assert!(matches!(&part.body, MediaBody::Bytes(bytes) if *bytes == JPEG));
    }
    {
        let mut files = Vec::new();
        add_image_to_files(&mut files, "input_reference", MediaBody::Bytes(GIF));
        assert_eq!(files.first().expect("part").content_type, "image/gif");
    }
    {
        let mut files = Vec::new();
        add_image_to_files(&mut files, "input_reference", MediaBody::Bytes(WEBP));
        assert_eq!(files.first().expect("part").content_type, "image/webp");
    }
    {
        let mut files = Vec::new();
        add_image_to_files(&mut files, "input_reference", MediaBody::Bytes(HEIC));
        assert_eq!(files.first().expect("part").content_type, "image/heic");
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
    let request = json!({
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
    });
    let mut request_map = request.as_object().cloned().expect("object");
    rewrite_character_ids(&mut request_map);
    assert_eq!(request_map.get("model"), Some(&json!("sora-2")));
    assert_eq!(request_map.get("prompt"), Some(&json!("make a video")));
    assert_eq!(request_map.get("seconds"), Some(&json!("4")));
    let characters = request_map
        .get("characters")
        .and_then(Value::as_array)
        .expect("list");
    assert_eq!(
        characters.first().and_then(|item| item.get("id")),
        Some(&json!("char-1"))
    );
    assert_eq!(
        characters.first().and_then(|item| item.get("role")),
        Some(&json!("hero"))
    );
    assert_eq!(
        characters.get(1).and_then(|item| item.get("role")),
        Some(&json!("plain"))
    );
    assert_eq!(
        characters.get(2).and_then(|item| item.get("id")),
        Some(&json!(7))
    );
    assert_eq!(characters.get(3), Some(&json!("raw")));
    assert_eq!(characters.get(4), Some(&json!(3)));
    let mut plain = json!({"model": "m", "characters": "none"})
        .as_object()
        .cloned()
        .expect("object");
    rewrite_character_ids(&mut plain);
    assert_eq!(plain.get("characters"), Some(&json!("none")));
}

fn video_filename_mime() {
    {
        let mut reader = ProbeReader::new(b"no signature", 0, None);
        let part = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.webm"),
                reader: &mut reader,
            },
            "video",
        );
        assert_eq!(part.field, "video");
        assert_eq!(part.filename, "clip.webm");
        assert_eq!(part.content_type, "video/webm");
    }
    let mut ftyp = vec![0_u8; 80];
    ftyp.get_mut(10..14)
        .expect("range")
        .copy_from_slice(b"ftyp");
    {
        let mut reader = ProbeReader::new(&ftyp, 0, None);
        let part = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.png"),
                reader: &mut reader,
            },
            "video",
        );
        assert_eq!(part.content_type, "video/mp4");
    }
    {
        let mut reader = ProbeReader::new(b"no ftyp", 0, None);
        let part = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.png"),
                reader: &mut reader,
            },
            "video",
        );
        assert_eq!(part.content_type, "video/mp4");
    }
    {
        let part = video_file_tuple(MediaBody::Bytes(b"no name"), "video");
        assert_eq!(part.filename, "input_video.mp4");
        assert_eq!(part.content_type, "video/mp4");
    }
    {
        let part = video_file_tuple(MediaBody::Text("a string"), "video");
        assert_eq!(part.filename, "input_video.mp4");
        assert_eq!(part.content_type, "video/mp4");
    }
    {
        let mut reader = ProbeReader::new(b"", 0, None);
        let part = video_file_tuple(
            MediaBody::Stream {
                name: Some(""),
                reader: &mut reader,
            },
            "video",
        );
        assert_eq!(part.filename, "input_video.mp4");
    }
}

fn video_probe_cursor() {
    let mut ftyp = vec![0_u8; 80];
    ftyp.get_mut(10..14)
        .expect("range")
        .copy_from_slice(b"ftyp");
    {
        let mut reader = ProbeReader::new(&ftyp, 3, None);
        let content_type = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.bin"),
                reader: &mut reader,
            },
            "video",
        )
        .content_type;
        assert_eq!(content_type, "video/mp4");
        assert_eq!(reader.position(), 3);
    }
    {
        let mut reader = ProbeReader::new(&ftyp, 2, None);
        let content_type = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.bin"),
                reader: &mut reader,
            },
            "video",
        )
        .content_type;
        assert_eq!(content_type, "video/mp4");
        assert_eq!(reader.position(), 2);
    }
    {
        let mut reader = ProbeReader::new(&ftyp, 3, Some(Failure::Tell));
        let content_type = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.bin"),
                reader: &mut reader,
            },
            "video",
        )
        .content_type;
        assert_eq!(content_type, "video/mp4");
        assert_eq!(reader.position(), 3);
    }
    {
        let mut reader = ProbeReader::new(&ftyp, 3, Some(Failure::Seek));
        let content_type = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.bin"),
                reader: &mut reader,
            },
            "video",
        )
        .content_type;
        assert_eq!(content_type, "video/mp4");
        assert_eq!(reader.position(), 3);
    }
    {
        let mut reader = ProbeReader::new(&ftyp, 3, Some(Failure::Read));
        let content_type = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.bin"),
                reader: &mut reader,
            },
            "video",
        )
        .content_type;
        assert_eq!(content_type, "video/mp4");
        assert_eq!(reader.position(), 0);
    }
    {
        let mut reader = ProbeReader::new(&ftyp, 3, Some(Failure::Restore));
        let content_type = video_file_tuple(
            MediaBody::Stream {
                name: Some("clip.bin"),
                reader: &mut reader,
            },
            "video",
        )
        .content_type;
        assert_eq!(content_type, "video/mp4");
        assert_eq!(reader.position(), 64);
    }
}
