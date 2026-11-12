use std::fs;
use std::path::PathBuf;
use std::str;

use rstest::rstest;

use crate::gemini_embedding_records::data_url::decode_embedding_data_url;
use crate::gemini_embedding_records::data_url::infer_mime_from_gcs_url;
use crate::gemini_embedding_records::error::GeminiEmbeddingError;

#[rstest]
#[case::png("data:image/png;base64,AAAA", "image/png", "AAAA")]
#[case::jpeg("data:image/jpeg;base64,/9j/4AAQ", "image/jpeg", "/9j/4AAQ")]
#[case::mp3("data:audio/mpeg;base64,SUQz", "audio/mpeg", "SUQz")]
#[case::wav("data:audio/wav;base64,UklGR", "audio/wav", "UklGR")]
#[case::mp4("data:video/mp4;base64,AAAA", "video/mp4", "AAAA")]
#[case::quicktime("data:video/quicktime;base64,AAAA", "video/quicktime", "AAAA")]
#[case::pdf("data:application/pdf;base64,JVBERi0", "application/pdf", "JVBERi0")]
#[case::parameters("data:image/png;charset=utf-8;base64,AAAA", "image/png", "AAAA")]
#[case::payload_commas("data:image/png;base64,AA,BB,CC", "image/png", "AA,BB,CC")]
#[case::empty_payload("data:image/png;base64,", "image/png", "")]
fn keeps_supported_media(#[case] source: &str, #[case] mime_type: &str, #[case] payload: &str) {
    let decoded = decode_embedding_data_url(source).unwrap();
    assert_eq!(decoded.mime_type, mime_type);
    assert_eq!(decoded.payload, payload);
}

#[rstest]
#[case::no_prefix("image/png;base64,AAAA")]
#[case::uppercase_prefix("DATA:image/png;base64,AAAA")]
#[case::empty("")]
fn rejects_without_the_data_prefix(#[case] source: &str) {
    assert!(matches!(
        decode_embedding_data_url(source),
        Err(GeminiEmbeddingError::DataUrlPrefix)
    ));
}

#[rstest]
#[case::no_comma("data:image/png;base64")]
#[case::prefix_only("data:")]
fn rejects_without_a_comma(#[case] source: &str) {
    assert!(matches!(
        decode_embedding_data_url(source),
        Err(GeminiEmbeddingError::DataUrlComma)
    ));
}

#[rstest]
#[case::text("data:text/plain;base64,AAAA", "text/plain")]
#[case::gif("data:image/gif;base64,AAAA", "image/gif")]
#[case::uppercase_media("data:IMAGE/PNG;base64,AAAA", "IMAGE/PNG")]
#[case::no_media("data:;base64,AAAA", "")]
#[case::parameters_only("data:;charset=utf-8,AAAA", "")]
fn rejects_unsupported_media(#[case] source: &str, #[case] mime_type: &str) {
    match decode_embedding_data_url(source) {
        Err(GeminiEmbeddingError::UnsupportedMime { mime_type: found }) => {
            assert_eq!(found, mime_type);
        }
        outcome => panic!("expected an unsupported media type, got {outcome:?}"),
    }
}

#[rstest]
#[case::png("gs://bucket/photo.png", "image/png")]
#[case::jpg("gs://bucket/photo.jpg", "image/jpeg")]
#[case::jpeg("gs://bucket/photo.JPEG", "image/jpeg")]
#[case::mp3("gs://bucket/audio.mp3", "audio/mpeg")]
#[case::wav("gs://bucket/audio.wav", "audio/wav")]
#[case::mp4("gs://bucket/clip.mp4", "video/mp4")]
#[case::mov("gs://bucket/clip.MoV", "video/quicktime")]
#[case::pdf("gs://bucket/doc.pdf", "application/pdf")]
fn infers_the_gcs_media_type(#[case] url: &str, #[case] mime_type: &str) {
    assert_eq!(infer_mime_from_gcs_url(url).unwrap(), mime_type);
}

#[rstest]
#[case::unknown_extension("gs://bucket/photo.gif")]
#[case::no_extension("gs://bucket/photo")]
#[case::suffix_after_extension("gs://bucket/photo.pngx")]
#[case::query_after_extension("gs://bucket/photo.png?x=1")]
#[case::trailing_slash("gs://bucket/photo.png/")]
fn rejects_unknown_gcs_extensions(#[case] url: &str) {
    assert!(matches!(
        infer_mime_from_gcs_url(url),
        Err(GeminiEmbeddingError::UnsupportedExtension { .. })
    ));
}

#[rstest]
fn replays_the_embedding_data_url_corpus(
    #[files("fuzz/corpus/embedding_data_url/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(source) = str::from_utf8(&bytes) else {
        return;
    };
    drop(decode_embedding_data_url(source));
}
