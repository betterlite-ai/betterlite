use std::io::Cursor;

use crate::openai_video_identifiers::multipart::MediaBody;
use crate::video_edits::character::{
    CharacterPart, CreateCharacter, transform_video_create_character_request,
};

fn part<'a>(created: &'a CreateCharacter<'_>) -> (&'a str, &'static str) {
    match created.files.get(1) {
        Some(CharacterPart::File(file)) => (file.filename.as_str(), file.content_type),
        _ => panic!("the second part is the video file"),
    }
}

fn created(video: MediaBody<'_>) -> CreateCharacter<'_> {
    transform_video_create_character_request("hero", video, "https://mock.example/v1")
}

pub(super) fn video_part_filename() {
    let mut named = Cursor::new(b"not video".to_vec());
    let stream = created(MediaBody::Stream {
        name: Some("clip.webm"),
        reader: &mut named,
    });
    assert_eq!(part(&stream).0, "clip.webm");

    let mut unnamed = Cursor::new(b"not video".to_vec());
    let stream = created(MediaBody::Stream {
        name: None,
        reader: &mut unnamed,
    });
    assert_eq!(part(&stream).0, "input_video.mp4");

    let mut empty = Cursor::new(b"not video".to_vec());
    let stream = created(MediaBody::Stream {
        name: Some(""),
        reader: &mut empty,
    });
    assert_eq!(
        part(&stream).0,
        "input_video.mp4",
        "an empty name is no name"
    );

    assert_eq!(
        part(&created(MediaBody::Bytes(b"not video"))).0,
        "input_video.mp4"
    );
    assert_eq!(
        part(&created(MediaBody::Text("a string"))).0,
        "input_video.mp4"
    );
}

pub(super) fn known_extension_mime() {
    let mut webm = Cursor::new(b"no signature".to_vec());
    let stream = created(MediaBody::Stream {
        name: Some("clip.webm"),
        reader: &mut webm,
    });
    assert_eq!(part(&stream), ("clip.webm", "video/webm"));

    let mut mp4 = Cursor::new(b"no signature".to_vec());
    let stream = created(MediaBody::Stream {
        name: Some("clip.mp4"),
        reader: &mut mp4,
    });
    assert_eq!(part(&stream), ("clip.mp4", "video/mp4"));
}

fn with_ftyp() -> Vec<u8> {
    let mut body = vec![0_u8; 80];
    body.get_mut(10..14)
        .expect("the ftyp range")
        .copy_from_slice(b"ftyp");
    body
}

pub(super) fn fallback_octet_stream() {
    let mut octet = Cursor::new(with_ftyp());
    let stream = created(MediaBody::Stream {
        name: Some("clip.bin"),
        reader: &mut octet,
    });
    assert_eq!(part(&stream), ("clip.bin", "video/mp4"));

    let mut image = Cursor::new(b"no ftyp".to_vec());
    let stream = created(MediaBody::Stream {
        name: Some("clip.png"),
        reader: &mut image,
    });
    assert_eq!(part(&stream), ("clip.png", "video/mp4"));

    assert_eq!(
        part(&created(MediaBody::Bytes(&with_ftyp()))).1,
        "video/mp4"
    );
}
