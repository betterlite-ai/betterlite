use betterlite_http::error::HttpError;

use crate::openai_video_identifiers::ids::encode_character_id;
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::video_edits::character::{
    CHARACTER_NAME_FIELD, CharacterPart, transform_video_create_character_request,
    transform_video_get_character_request,
};
use crate::video_edits::error::VideoEditError;
use crate::video_edits::response::{CharacterKind, decode_character_reply};

pub(super) fn character_create_get() {
    let created = transform_video_create_character_request(
        "hero",
        MediaBody::Bytes(b"not video"),
        "https://mock.example/v1/",
    );
    assert_eq!(created.url, "https://mock.example/v1/characters");
    let [
        CharacterPart::Text { field, text },
        CharacterPart::File(video),
    ] = created.files.as_slice()
    else {
        panic!("the files list is the name field and then the video");
    };
    assert_eq!(*field, CHARACTER_NAME_FIELD);
    assert_eq!(*text, "hero");
    assert_eq!(video.field, "video");
    assert_eq!(video.filename, "input_video.mp4");
    assert_eq!(video.content_type, "video/mp4");
    let MediaBody::Bytes(bytes) = &video.body else {
        panic!("the video part keeps the supplied bytes");
    };
    assert_eq!(*bytes, b"not video".as_slice());

    let encoded = encode_character_id("char-1", "openai", Some("sora-2"));
    let lookup = transform_video_get_character_request(&encoded, "https://mock.example/v1")
        .expect("the character id is a path segment");
    assert_eq!(lookup, "https://mock.example/v1/characters/char-1");
    let escaped = transform_video_get_character_request("a b/c+d", "https://mock.example/v1/")
        .expect("the character id is a path segment");
    assert_eq!(escaped, "https://mock.example/v1/characters/a%20b%2Fc%2Bd");
    let empty = transform_video_get_character_request("", "https://mock.example/v1")
        .expect_err("an empty character id has no path segment");
    let VideoEditError::CharacterId(HttpError::Required { field_name }) = empty else {
        panic!("an empty character id is a required field");
    };
    assert_eq!(field_name, "character_id");
    let dot = transform_video_get_character_request("..", "https://mock.example/v1")
        .expect_err("a dot character id has no path segment");
    let VideoEditError::CharacterId(HttpError::DotSegment { field_name }) = dot else {
        panic!("a dot character id is a dot segment");
    };
    assert_eq!(field_name, "character_id");

    let body = br#"{"id":"character_abc","object":"character","created_at":1700000000,"name":"hero","extra":1}"#;
    let object = decode_character_reply(body).expect("a character reply");
    assert_eq!(object.id, "character_abc");
    assert_eq!(object.object, CharacterKind::Character);
    assert_eq!(object.created_at, 1_700_000_000);
    assert_eq!(object.name, "hero");
    let defaulted = decode_character_reply(br#"{"id":"c1","created_at":5,"name":"n"}"#)
        .expect("the object field defaults to character");
    assert_eq!(defaulted.object, CharacterKind::Character);
    let wrong_object =
        decode_character_reply(br#"{"id":"c1","created_at":5,"name":"n","object":"video"}"#);
    assert!(wrong_object.is_err());
    let missing_name = decode_character_reply(br#"{"id":"c1","created_at":5}"#);
    assert!(missing_name.is_err());
    let wrong_type = decode_character_reply(br#"{"id":1,"created_at":5,"name":"n"}"#);
    assert!(wrong_type.is_err());
    let empty_body = decode_character_reply(b"");
    assert!(empty_body.is_err());
}
