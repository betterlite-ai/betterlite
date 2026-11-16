#![cfg(test)]

use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::media::data_url::MAX_MEDIA_BYTES;
use rstest::rstest;

use crate::vertex_imagen_edit::decode::{decode_imagen_edit_input, parse_imagen_edit_input};
use crate::vertex_imagen_edit::error::ImagenEditError;
use crate::vertex_imagen_edit::input::{
    ImagenFields, ImagenFileInput, ImagenFileRead, ImagenReadResult, read_image_bytes,
};

fn read_bytes(input: &mut ImagenFileInput<'_>) -> Result<Vec<u8>, ImagenEditError> {
    read_image_bytes(input, 0, 100)
}

fn document(body: &str) -> ImagenFileInput<'static> {
    parse_imagen_edit_input(body.as_bytes())
        .expect("the document parses")
        .expect("the document is an image record")
}

fn read_document(body: &str) -> Result<Vec<u8>, ImagenEditError> {
    read_image_bytes(&mut document(body), 0, 100)
}

// every level adds one to the depth its member is read at
fn nested(depth: usize, leaf: ImagenFileInput<'static>) -> ImagenFileInput<'static> {
    let mut image = leaf;
    for _ in 0..depth {
        image = ImagenFileInput::Sequence(vec![Some(image)]);
    }
    image
}

fn nested_mapping(depth: usize, leaf: ImagenFileInput<'static>) -> ImagenFileInput<'static> {
    let mut image = leaf;
    for _ in 0..depth {
        image = ImagenFileInput::Mapping(ImagenFields {
            data: None,
            bytes: None,
            content: None,
            path: Some(Box::new(image)),
        });
    }
    image
}

struct NoTellStream {
    content: Vec<u8>,
    position: usize,
}

impl Read for NoTellStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.content[self.position..];
        let count = remaining.len().min(buffer.len());
        buffer[..count].copy_from_slice(&remaining[..count]);
        self.position = self.position.saturating_add(count);
        Ok(count)
    }
}

// a tell is a seek to the current position, which this stream refuses
impl Seek for NoTellStream {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match position {
            SeekFrom::Start(offset) => {
                self.position = usize::try_from(offset).expect("a seek offset fits");
                Ok(offset)
            }
            SeekFrom::Current(_) | SeekFrom::End(_) => Err(io::Error::other("tell refused")),
        }
    }
}

// the tell answers, while the rewind from a moved cursor does not
struct NoRewindStream {
    content: Vec<u8>,
    position: usize,
}

impl Read for NoRewindStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.content[self.position..];
        let count = remaining.len().min(buffer.len());
        buffer[..count].copy_from_slice(&remaining[..count]);
        self.position = self.position.saturating_add(count);
        Ok(count)
    }
}

impl Seek for NoRewindStream {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match position {
            SeekFrom::Current(0) => Ok(u64::try_from(self.position).expect("a position fits")),
            SeekFrom::Start(0) => Err(io::Error::other("rewind refused")),
            SeekFrom::Start(offset) => {
                self.position = usize::try_from(offset).expect("a seek offset fits");
                Ok(offset)
            }
            SeekFrom::Current(_) | SeekFrom::End(_) => Err(io::Error::other("seek refused")),
        }
    }
}

// tell and rewind answer, while the read does not
struct NoReadStream;

impl Read for NoReadStream {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("read refused"))
    }
}

impl Seek for NoReadStream {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Ok(0)
    }
}

struct BytesReader(Vec<u8>);

impl ImagenFileRead for BytesReader {
    fn read_all(&mut self) -> io::Result<ImagenReadResult> {
        Ok(ImagenReadResult::Bytes(self.0.clone()))
    }
}

struct TextReader(String);

impl ImagenFileRead for TextReader {
    fn read_all(&mut self) -> io::Result<ImagenReadResult> {
        Ok(ImagenReadResult::Text(self.0.clone()))
    }
}

struct BrokenReader;

impl ImagenFileRead for BrokenReader {
    fn read_all(&mut self) -> io::Result<ImagenReadResult> {
        Err(io::Error::other("read refused"))
    }
}

#[test]
fn the_shared_depth_limit_is_one_hundred_levels() {
    let limit = Settings::default().max_recurse_depth;
    let levels = usize::try_from(limit).expect("the default limit fits");
    let mut at_limit = nested(levels, ImagenFileInput::Bytes(b"abc".to_vec()));
    assert_eq!(
        read_image_bytes(&mut at_limit, 0, limit).expect("a hundred levels are read"),
        b"abc".as_slice()
    );
    let mut past_limit = nested(
        levels.saturating_add(1),
        ImagenFileInput::Bytes(b"abc".to_vec()),
    );
    let error = read_image_bytes(&mut past_limit, 0, limit).expect_err("a hundred and one are not");
    assert!(matches!(error, ImagenEditError::Depth { max_depth: 100 }));
}

#[test]
fn nested_mappings_count_toward_the_depth_limit() {
    let limit = Settings::default().max_recurse_depth;
    let levels = usize::try_from(limit).expect("the default limit fits");
    let mut at_limit = nested_mapping(levels, ImagenFileInput::Bytes(b"abc".to_vec()));
    assert_eq!(
        read_image_bytes(&mut at_limit, 0, limit).expect("a hundred levels are read"),
        b"abc".as_slice()
    );
    let mut past_limit = nested_mapping(
        levels.saturating_add(1),
        ImagenFileInput::Bytes(b"abc".to_vec()),
    );
    let error = read_image_bytes(&mut past_limit, 0, limit).expect_err("a hundred and one are not");
    assert!(matches!(error, ImagenEditError::Depth { max_depth: 100 }));
}

#[test]
fn a_bytes_or_bytearray_record_is_used_as_it_came() {
    let mut content = ImagenFileInput::Bytes(b"abc".to_vec());
    assert_eq!(
        read_bytes(&mut content).expect("bytes read"),
        b"abc".as_slice()
    );
    let mut array = ImagenFileInput::ByteArray(b"abc".to_vec());
    assert_eq!(
        read_bytes(&mut array).expect("a bytearray reads"),
        b"abc".as_slice()
    );
}

#[test]
fn a_text_or_path_record_is_refused() {
    let mut text = ImagenFileInput::Text("YWJj".to_owned());
    let error = read_bytes(&mut text).expect_err("a bare string is refused");
    assert!(matches!(error, ImagenEditError::PlainString));
    assert!(error.upstream_answer().is_none());
    let mut path = ImagenFileInput::Path(PathBuf::from("image.png"));
    let path_error = read_bytes(&mut path).expect_err("a path is refused");
    assert!(matches!(path_error, ImagenEditError::FilePath));
}

#[test]
fn a_sequence_reads_only_its_first_non_none_member() {
    let mut image = ImagenFileInput::Sequence(vec![
        None,
        Some(ImagenFileInput::Bytes(b"abc".to_vec())),
        Some(ImagenFileInput::Bytes(b"zzz".to_vec())),
    ]);
    assert_eq!(
        read_bytes(&mut image).expect("the first member answers"),
        b"abc".as_slice()
    );
}

#[test]
fn a_sequence_whose_first_member_is_a_string_is_refused() {
    let mut image =
        ImagenFileInput::Sequence(vec![None, Some(ImagenFileInput::Text("YWJj".to_owned()))]);
    let error = read_bytes(&mut image).expect_err("a bare string member is refused");
    assert!(matches!(error, ImagenEditError::PlainString));
}

#[test]
fn a_sequence_without_a_member_is_unsupported() {
    let mut empty = ImagenFileInput::Sequence(Vec::new());
    let error = read_bytes(&mut empty).expect_err("an empty sequence holds no image");
    assert!(matches!(
        error,
        ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member"
        }
    ));
    let mut all_none = ImagenFileInput::Sequence(vec![None, None]);
    let all_none_error = read_bytes(&mut all_none).expect_err("a list of Nones holds no image");
    assert!(matches!(
        all_none_error,
        ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member"
        }
    ));
}

#[test]
fn a_mapping_prefers_data_then_bytes_then_content() {
    let mut all = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Bytes(b"data".to_vec()))),
        bytes: Some(Box::new(ImagenFileInput::Bytes(b"bytes".to_vec()))),
        content: Some(Box::new(ImagenFileInput::Bytes(b"content".to_vec()))),
        path: Some(Box::new(ImagenFileInput::Path(PathBuf::from("image.png")))),
    });
    assert_eq!(
        read_bytes(&mut all).expect("data answers"),
        b"data".as_slice()
    );
    let mut without_data = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: Some(Box::new(ImagenFileInput::Bytes(b"bytes".to_vec()))),
        content: Some(Box::new(ImagenFileInput::Bytes(b"content".to_vec()))),
        path: Some(Box::new(ImagenFileInput::Path(PathBuf::from("image.png")))),
    });
    assert_eq!(
        read_bytes(&mut without_data).expect("bytes answer"),
        b"bytes".as_slice()
    );
    let mut content_only = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: Some(Box::new(ImagenFileInput::Bytes(b"content".to_vec()))),
        path: Some(Box::new(ImagenFileInput::Path(PathBuf::from("image.png")))),
    });
    assert_eq!(
        read_bytes(&mut content_only).expect("content answers"),
        b"content".as_slice()
    );
}

#[test]
fn a_mapping_decodes_a_base64_string_in_its_first_key() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Text(
            "MDEyMzQ1Njc4OQ==".to_owned(),
        ))),
        bytes: None,
        content: None,
        path: None,
    });
    assert_eq!(
        read_bytes(&mut image).expect("the base64 text decodes"),
        b"0123456789".as_slice()
    );
}

#[test]
fn a_mapping_falls_through_a_base64_string_it_cannot_decode() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Text("not base64!".to_owned()))),
        bytes: Some(Box::new(ImagenFileInput::Text("YWJj".to_owned()))),
        content: None,
        path: None,
    });
    assert_eq!(
        read_bytes(&mut image).expect("the next key answers"),
        b"abc".as_slice()
    );
}

#[test]
fn a_mapping_reads_its_path_after_the_value_keys() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: Some(Box::new(ImagenFileInput::Sequence(vec![Some(
            ImagenFileInput::Bytes(b"abc".to_vec()),
        )]))),
    });
    assert_eq!(
        read_bytes(&mut image).expect("the path recurses"),
        b"abc".as_slice()
    );
    let mut string_path = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: Some(Box::new(ImagenFileInput::Text("image.png".to_owned()))),
    });
    let error = read_bytes(&mut string_path).expect_err("a string path is refused");
    assert!(matches!(error, ImagenEditError::PlainString));
}

#[test]
fn a_mapping_without_an_image_key_is_unsupported() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: None,
    });
    let error = read_bytes(&mut image).expect_err("a mapping of nothing holds no image");
    assert!(matches!(
        error,
        ImagenEditError::UnsupportedImage {
            kind: "a mapping without an image key"
        }
    ));
}

#[test]
fn an_unsupported_record_names_its_kind() {
    let mut image = ImagenFileInput::Unsupported { kind: "a number" };
    let error = read_bytes(&mut image).expect_err("a number holds no image");
    assert!(matches!(
        error,
        ImagenEditError::UnsupportedImage { kind: "a number" }
    ));
}

#[test]
fn a_seekable_stream_is_read_whole_and_left_where_it_was() {
    let mut content = Cursor::new(b"0123456789abcdef".to_vec());
    content.seek(SeekFrom::Start(7)).expect("the cursor moves");
    let mut image = ImagenFileInput::Seekable(&mut content);
    assert_eq!(
        read_bytes(&mut image).expect("the stream reads"),
        b"0123456789abcdef".as_slice()
    );
    assert_eq!(content.position(), 7);
}

#[test]
fn a_seekable_stream_whose_tell_refuses_is_a_stream_error() {
    let mut stream = NoTellStream {
        content: b"0123456789".to_vec(),
        position: 7,
    };
    let mut image = ImagenFileInput::Seekable(&mut stream);
    let error = read_bytes(&mut image).expect_err("a BytesIO always answers its tell");
    assert!(matches!(error, ImagenEditError::Stream(_)));
}

#[test]
fn a_seekable_stream_that_cannot_rewind_is_a_stream_error() {
    let mut stream = NoRewindStream {
        content: b"0123456789".to_vec(),
        position: 7,
    };
    let mut image = ImagenFileInput::Seekable(&mut stream);
    let error = read_bytes(&mut image).expect_err("a BytesIO always rewinds");
    assert!(matches!(error, ImagenEditError::Stream(_)));
}

#[test]
fn a_seekable_stream_that_cannot_read_is_a_stream_error() {
    let mut stream = NoReadStream;
    let mut image = ImagenFileInput::Seekable(&mut stream);
    let error = read_bytes(&mut image).expect_err("a refused read fails");
    assert!(matches!(error, ImagenEditError::Stream(_)));
}

#[test]
fn a_buffered_stream_with_a_position_is_rewound_and_restored() {
    let mut content = Cursor::new(b"0123456789abcdef".to_vec());
    content.seek(SeekFrom::Start(7)).expect("the cursor moves");
    let mut image = ImagenFileInput::Buffered(&mut content);
    assert_eq!(
        read_bytes(&mut image).expect("the stream reads"),
        b"0123456789abcdef".as_slice()
    );
    assert_eq!(content.position(), 7);
}

#[test]
fn a_buffered_stream_that_cannot_tell_is_read_from_where_it_stands() {
    let mut stream = NoTellStream {
        content: b"0123456789".to_vec(),
        position: 7,
    };
    let mut image = ImagenFileInput::Buffered(&mut stream);
    assert_eq!(
        read_bytes(&mut image).expect("the stream reads"),
        b"789".as_slice()
    );
}

#[test]
fn a_reader_answers_its_bytes_or_its_text() {
    let mut bytes = BytesReader(b"abc".to_vec());
    let mut image = ImagenFileInput::Reader(&mut bytes);
    assert_eq!(
        read_bytes(&mut image).expect("the reader answers"),
        b"abc".as_slice()
    );
    let mut text = TextReader("héllo".to_owned());
    let mut text_image = ImagenFileInput::Reader(&mut text);
    assert_eq!(
        read_bytes(&mut text_image).expect("the text is encoded"),
        "héllo".as_bytes()
    );
}

#[test]
fn a_reader_that_fails_is_a_stream_error() {
    let mut stream = BrokenReader;
    let mut image = ImagenFileInput::Reader(&mut stream);
    let error = read_bytes(&mut image).expect_err("a refused read fails");
    assert!(matches!(error, ImagenEditError::Stream(_)));
}

#[rstest]
#[case::padded("YWJj", b"abc")]
#[case::junk_between_symbols("YW Jj\n", b"abc")]
#[case::padding("YQ==", b"a")]
#[case::missing_padding("MDEy", b"012")]
#[case::empty("", b"")]
#[case::data_after_a_pad_run_is_decoded("QQ==QUJDRA==", b"A\x04\x14$4@")]
#[case::pads_after_a_whole_quad_are_skipped("QUJD====QUJD", b"ABCABC")]
#[case::pads_after_one_symbol_are_skipped("Q===QUJ", b"A\x05\x09")]
#[case::a_leading_pad_is_skipped("=YQ==", b"a")]
#[case::a_symbol_restarts_the_pad_count("YQ=YWJj=Y=", b"a\x06\x16&6")]
fn permissive_base64_decodes(#[case] text: &str, #[case] expected: &[u8]) {
    assert_eq!(
        decode_imagen_edit_input(text).expect("the text decodes"),
        expected
    );
}

#[rstest]
#[case::one_symbol("Y")]
#[case::two_symbols("YW")]
#[case::three_symbols("YWJ")]
#[case::junk_symbols("not base64!")]
#[case::non_ascii("héllo")]
#[case::a_short_pad_run_leaves_a_partial_quad("YQ=Q")]
#[case::non_ascii_after_a_pad_run("YQ==é")]
#[case::a_pad_run_that_data_follows_leaves_a_partial_quad("QUI=QUJD")]
#[case::pads_split_by_junk_do_not_complete_the_quad("YQ=\n=QUJD")]
fn an_undecodable_text_is_a_base64_error(#[case] text: &str) {
    let error = decode_imagen_edit_input(text).expect_err("the text does not decode");
    assert!(matches!(error, ImagenEditError::Base64(_)));
}

#[test]
fn an_input_past_the_ceiling_is_refused() {
    let oversized = "A".repeat(MAX_MEDIA_BYTES.saturating_add(1));
    let text_error =
        decode_imagen_edit_input(&oversized).expect_err("the text is past the ceiling");
    assert!(matches!(
        text_error,
        ImagenEditError::TooLarge { size } if size == MAX_MEDIA_BYTES.saturating_add(1)
    ));
    let body_error = parse_imagen_edit_input(oversized.as_bytes())
        .err()
        .expect("the body is past it");
    assert!(matches!(
        body_error,
        ImagenEditError::TooLarge { size } if size == MAX_MEDIA_BYTES.saturating_add(1)
    ));
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Text(oversized))),
        bytes: None,
        content: None,
        path: None,
    });
    let field_error = read_bytes(&mut image).expect_err("the data text is past the ceiling");
    assert!(matches!(
        field_error,
        ImagenEditError::TooLarge { size } if size == MAX_MEDIA_BYTES.saturating_add(1)
    ));
}

#[test]
fn an_input_at_the_ceiling_passes_it() {
    let bounded = "A".repeat(MAX_MEDIA_BYTES);
    assert_eq!(
        decode_imagen_edit_input(&bounded)
            .expect("the text decodes")
            .len(),
        MAX_MEDIA_BYTES.saturating_div(4).saturating_mul(3)
    );
    let error = parse_imagen_edit_input(bounded.as_bytes())
        .err()
        .expect("the body is not JSON");
    assert!(matches!(error, ImagenEditError::InputJson(_)));
}

#[test]
fn a_null_document_is_no_image() {
    assert!(
        parse_imagen_edit_input(b"null")
            .expect("null parses")
            .is_none()
    );
}

#[test]
fn a_document_that_is_not_json_is_an_input_error() {
    let error = parse_imagen_edit_input(b"{\"data\":")
        .err()
        .expect("a truncated document fails");
    assert!(matches!(error, ImagenEditError::InputJson(_)));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_document_refuses_what_the_record_refuses() {
    assert!(matches!(
        read_document(r#""plain""#),
        Err(ImagenEditError::PlainString)
    ));
    assert!(matches!(
        read_document("7"),
        Err(ImagenEditError::UnsupportedImage { kind: "a number" })
    ));
    assert!(matches!(
        read_document("true"),
        Err(ImagenEditError::UnsupportedImage { kind: "a boolean" })
    ));
    assert!(matches!(
        read_document(r#"["YWJj", 5]"#),
        Err(ImagenEditError::PlainString)
    ));
    assert!(matches!(
        read_document("[]"),
        Err(ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member"
        })
    ));
    assert!(matches!(
        read_document("{}"),
        Err(ImagenEditError::UnsupportedImage {
            kind: "a mapping without an image key"
        })
    ));
}

#[test]
fn a_document_path_is_refused_like_the_record() {
    assert!(matches!(
        read_document(r#"{"path":"x.png"}"#),
        Err(ImagenEditError::PlainString)
    ));
    assert!(matches!(
        read_document(r#"{"path":["YWJj"]}"#),
        Err(ImagenEditError::PlainString)
    ));
}

#[test]
fn a_document_reads_the_base64_in_its_value_keys() {
    assert_eq!(
        read_document(r#"{"data":"YWJj"}"#).expect("the data decodes"),
        b"abc".as_slice()
    );
    assert_eq!(
        read_document(r#"{"data":"YQ=","bytes":"YWJj"}"#).expect("the bytes answer"),
        b"abc".as_slice()
    );
    assert_eq!(
        read_document(r#"{"data":{"data":"YWJj"}}"#).expect("the nested mapping answers"),
        b"abc".as_slice()
    );
}

#[rstest]
fn replays_the_imagen_edit_input_corpus(#[files("fuzz/corpus/imagen_edit_input/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus file reads");
    let Ok(projected) = parse_imagen_edit_input(&bytes) else {
        return;
    };
    let Some(mut image) = projected else {
        return;
    };
    drop(read_image_bytes(
        &mut image,
        0,
        Settings::default().max_recurse_depth,
    ));
}
