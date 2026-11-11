#![cfg(test)]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::media_source::{
    MAX_MEDIA_BYTES, MediaSizeGuard, content_length_within_limit, media_duration_seconds,
    media_predates_job, s3_media_url,
};

const VALID_WAV: &[u8] = include_bytes!("../../fuzz/corpus/media_duration/valid.wav");
const VALID_FLAC: &[u8] = include_bytes!("../../fuzz/corpus/media_duration/valid.flac");
const VALID_OGG: &[u8] = include_bytes!("../../fuzz/corpus/media_duration/valid.ogg");
const VALID_MP3: &[u8] = include_bytes!("../../fuzz/corpus/media_duration/valid.mp3");

// 2015-10-21T07:28:00Z, the creation time the Last-Modified cases compare against
const JOB_CREATED_AT: f64 = 1_445_412_480.0;

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn wav_bytes(sample_rate: u32, frames: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    for _frame in 0..frames {
        writer.write_sample(0_i16).unwrap();
    }
    writer.finalize().unwrap();
    cursor.into_inner()
}

fn zero_rate_wav() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&36_u32.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}

#[test]
fn media_url_builds_a_virtual_host_url_for_a_dotless_bucket() {
    assert_eq!(
        s3_media_url("s3://audio/meeting.wav", "us-east-1").unwrap(),
        Some("https://audio.s3.us-east-1.amazonaws.com/meeting.wav".to_owned())
    );
}

#[test]
fn media_url_builds_a_path_style_url_for_a_dotted_bucket() {
    assert_eq!(
        s3_media_url("s3://my.audio/a.wav", "eu-west-1").unwrap(),
        Some("https://s3.eu-west-1.amazonaws.com/my.audio/a.wav".to_owned())
    );
}

#[test]
fn media_url_keeps_a_missing_key_as_a_trailing_slash() {
    assert_eq!(
        s3_media_url("s3://audio", "us-east-1").unwrap(),
        Some("https://audio.s3.us-east-1.amazonaws.com/".to_owned())
    );
}

#[test]
fn media_url_percent_encodes_the_key_like_python_quote() {
    assert_eq!(
        s3_media_url("s3://audio/folder/my file (1).wav?next=a=b", "us-east-1")
            .unwrap(),
        Some(
            "https://audio.s3.us-east-1.amazonaws.com/folder/my%20file%20%281%29.wav%3Fnext%3Da%3Db"
                .to_owned()
        )
    );
    assert_eq!(
        s3_media_url("s3://audio/\u{fc}.wav", "us-east-1").unwrap(),
        Some("https://audio.s3.us-east-1.amazonaws.com/%C3%BC.wav".to_owned())
    );
}

#[rstest]
#[case::same_partition("https://audio.s3.us-east-1.amazonaws.com/a.wav", "us-east-1", true)]
#[case::path_style("https://s3.us-east-1.amazonaws.com/audio/a.wav", "us-east-1", true)]
#[case::foreign_host("https://files.example.org/a.wav", "us-east-1", false)]
#[case::plain_root("https://amazonaws.com/a.wav", "us-east-1", false)]
#[case::http_scheme("http://audio.s3.us-east-1.amazonaws.com/a.wav", "us-east-1", false)]
#[case::china_partition(
    "https://audio.s3.cn-north-1.amazonaws.com.cn/a.wav",
    "cn-north-1",
    true
)]
#[case::commercial_url_in_china(
    "https://audio.s3.us-east-1.amazonaws.com/a.wav",
    "cn-north-1",
    false
)]
fn media_url_passes_through_only_https_urls_on_the_partition_suffix(
    #[case] media_uri: &str,
    #[case] region: &str,
    #[case] accepted: bool,
) {
    let expected = accepted.then(|| media_uri.to_owned());
    assert_eq!(s3_media_url(media_uri, region).unwrap(), expected);
}

#[rstest]
#[case::plain_text("not a url")]
#[case::empty("")]
#[case::missing_host("https://")]
fn media_url_errors_for_a_string_that_is_not_a_url(#[case] media_uri: &str) {
    let error = s3_media_url(media_uri, "us-east-1").unwrap_err();
    assert!(error.to_string().contains("URL"));
}

#[rstest]
#[case::same_second("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT", true)]
#[case::one_second_later("last-modified", "Wed, 21 Oct 2015 07:28:01 GMT", true)]
#[case::one_second_earlier("last-modified", "Wed, 21 Oct 2015 07:27:59 GMT", true)]
#[case::capitalized_name("Last-Modified", "Wed, 21 Oct 2015 07:28:01 GMT", true)]
#[case::two_seconds_later("last-modified", "Wed, 21 Oct 2015 07:28:02 GMT", false)]
#[case::utc_zone("last-modified", "Wed, 21 Oct 2015 07:28:01 +0000", true)]
#[case::negative_zero_zone("last-modified", "Wed, 21 Oct 2015 07:28:01 -0000", true)]
fn last_modified_compares_the_header_with_the_job_creation_time(
    #[case] name: &str,
    #[case] value: &str,
    #[case] predates: bool,
) {
    assert_eq!(
        media_predates_job(&headers(&[(name, value)]), JOB_CREATED_AT),
        predates
    );
}

#[test]
fn last_modified_compares_against_a_fractional_creation_time() {
    let created_at = 1_445_412_480.5_f64;
    assert!(media_predates_job(
        &headers(&[("last-modified", "Wed, 21 Oct 2015 07:28:01 GMT")]),
        created_at
    ));
    assert!(!media_predates_job(
        &headers(&[("last-modified", "Wed, 21 Oct 2015 07:28:02 GMT")]),
        created_at
    ));
}

#[test]
fn last_modified_is_false_without_the_header() {
    assert!(!media_predates_job(&[], JOB_CREATED_AT));
    assert!(!media_predates_job(
        &headers(&[("etag", "abc")]),
        JOB_CREATED_AT
    ));
}

#[rstest]
#[case::words("yesterday")]
#[case::no_zone("Wed, 21 Oct 2015 07:28:00")]
#[case::empty("")]
fn last_modified_is_false_for_an_unparsable_header(#[case] value: &str) {
    assert!(!media_predates_job(
        &headers(&[("last-modified", value)]),
        JOB_CREATED_AT
    ));
}

#[test]
fn size_limit_rejects_a_content_length_above_the_cap() {
    assert!(!content_length_within_limit(&headers(&[("Content-Length", "2147483649")])).unwrap());
}

#[test]
fn size_limit_allows_a_content_length_at_the_cap() {
    assert!(content_length_within_limit(&headers(&[("content-length", "2147483648")])).unwrap());
    assert_eq!(MAX_MEDIA_BYTES, 0x8000_0000);
}

#[test]
fn size_limit_reads_a_missing_content_length_as_zero() {
    assert!(content_length_within_limit(&[]).unwrap());
    assert!(content_length_within_limit(&headers(&[("etag", "abc")])).unwrap());
}

#[rstest]
#[case::words("2 GiB")]
#[case::fraction("1.5")]
#[case::negative("-1")]
#[case::empty("")]
fn size_limit_errors_on_a_content_length_that_is_not_an_integer(#[case] value: &str) {
    let error = content_length_within_limit(&headers(&[("content-length", value)])).unwrap_err();
    assert!(error.to_string().contains("content-length"));
}

#[test]
fn size_limit_stops_streamed_bytes_past_the_cap() {
    let mut guard = MediaSizeGuard::default();
    let cap = usize::try_from(MAX_MEDIA_BYTES).unwrap();
    assert!(guard.accept(cap));
    assert!(!guard.accept(1));
}

#[test]
fn size_limit_refuses_one_chunk_past_the_cap() {
    let mut guard = MediaSizeGuard::default();
    let over = usize::try_from(MAX_MEDIA_BYTES).unwrap().saturating_add(1);
    assert!(!guard.accept(over));
}

#[test]
fn media_duration_reads_a_wav_frames_over_sample_rate() {
    assert_eq!(
        media_duration_seconds(Cursor::new(VALID_WAV)),
        Some(Decimal::ONE)
    );
    assert_eq!(
        media_duration_seconds(Cursor::new(wav_bytes(8000, 12000))),
        Some(Decimal::new(15, 1))
    );
}

#[test]
fn media_duration_reads_a_flac_streaminfo() {
    assert_eq!(
        media_duration_seconds(Cursor::new(VALID_FLAC)),
        Some(Decimal::ONE)
    );
}

#[test]
fn media_duration_reads_an_ogg_last_page_granule() {
    assert_eq!(
        media_duration_seconds(Cursor::new(VALID_OGG)),
        Some(Decimal::ONE)
    );
}

#[test]
fn media_duration_reads_an_mp3_frame_count() {
    // the Xing header declares 16 frames of 576 samples at 8 kHz, and the LAME tag trims the encoder delay and padding to one second
    assert_eq!(
        media_duration_seconds(Cursor::new(VALID_MP3)),
        Some(Decimal::ONE)
    );
}

#[test]
fn media_duration_skips_an_id3v2_tag_larger_than_the_probe_depth() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ID3");
    bytes.extend_from_slice(&[4, 0, 0]); // version 2.4, no footer flag
    bytes.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // synchsafe 2 MiB, past the 1 MiB probe depth
    bytes.resize(bytes.len() + 2 * 1024 * 1024, 0);
    bytes.extend_from_slice(VALID_MP3);
    assert_eq!(
        media_duration_seconds(Cursor::new(bytes)),
        Some(Decimal::ONE)
    );
}

#[test]
fn media_duration_is_none_for_an_unknown_container() {
    assert_eq!(
        media_duration_seconds(Cursor::new(b"not audio at all".as_slice())),
        None
    );
    assert_eq!(media_duration_seconds(Cursor::new(Vec::new())), None);
}

#[rstest]
#[case::wav(&VALID_WAV[..40])]
#[case::flac(&VALID_FLAC[..20])]
#[case::ogg(&VALID_OGG[..5])]
#[case::mp3(&VALID_MP3[..2])]
fn media_duration_is_none_for_truncated_input(#[case] bytes: &[u8]) {
    assert_eq!(media_duration_seconds(Cursor::new(bytes)), None);
}

#[test]
fn media_duration_is_none_for_a_zero_sample_rate() {
    assert_eq!(media_duration_seconds(Cursor::new(zero_rate_wav())), None);
}

#[rstest]
fn replays_the_media_duration_corpus(#[files("fuzz/corpus/media_duration/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _seconds = media_duration_seconds(Cursor::new(bytes));
}
