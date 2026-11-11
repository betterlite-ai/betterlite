#![cfg(test)]

use std::fs;
use std::io::{self, Cursor, Read};
use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::media_source::mp3::mp3_seconds;
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

const STEREO_FRAME: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];
const STEREO_FRAME_BYTES: usize = 417;

// nonzero filler, so a frame length that is off by one misaligns the next header
fn frame(header: [u8; 4], length: usize) -> Vec<u8> {
    let mut bytes = vec![0x01_u8; length];
    bytes[..4].copy_from_slice(&header);
    bytes
}

fn xing_frame(
    header: [u8; 4],
    length: usize,
    side_information: usize,
    tag: [u8; 4],
    (flags, frames): (u32, u32),
) -> Vec<u8> {
    let mut bytes = frame(header, length);
    let fields = [
        tag.as_slice(),
        flags.to_be_bytes().as_slice(),
        frames.to_be_bytes().as_slice(),
    ]
    .concat();
    for (slot, value) in bytes.iter_mut().skip(4).skip(side_information).zip(fields) {
        *slot = value;
    }
    bytes
}

struct FailingAfter {
    data: Cursor<Vec<u8>>,
}

impl Read for FailingAfter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.data.read(buf)?;
        if read == 0 && !buf.is_empty() {
            return Err(io::Error::other("the source went away"));
        }
        Ok(read)
    }
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
    // the Xing header declares 16 frames of 576 samples at 8 kHz
    assert_eq!(
        media_duration_seconds(Cursor::new(VALID_MP3)),
        Some(Decimal::new(1152, 3))
    );
}

#[rstest]
#[case::v1_layer3_44k([0xFF, 0xFB, 0x90, 0x00], 417, 2304, 44_100)]
#[case::v1_layer3_48k([0xFF, 0xFB, 0x94, 0x00], 384, 2304, 48_000)]
#[case::v1_layer3_32k([0xFF, 0xFB, 0x98, 0x00], 576, 2304, 32_000)]
#[case::v1_layer2([0xFF, 0xFD, 0x10, 0x00], 104, 2304, 44_100)]
#[case::v1_layer1([0xFF, 0xFF, 0x40, 0x00], 139, 768, 44_100)]
#[case::v2_layer3([0xFF, 0xF3, 0x80, 0x00], 208, 1152, 22_050)]
#[case::v2_layer2([0xFF, 0xF5, 0x18, 0x00], 72, 2304, 16_000)]
#[case::v25_layer3_mono([0xFF, 0xE3, 0x88, 0xC0], 576, 1152, 8_000)]
#[case::v25_layer2([0xFF, 0xE5, 0x40, 0x00], 417, 2304, 11_025)]
fn media_duration_measures_a_frame_of_every_version_and_layer(
    #[case] header: [u8; 4],
    #[case] length: usize,
    #[case] samples_of_both: u64,
    #[case] sample_rate: u32,
) {
    let bytes = [frame(header, length), frame(header, length)].concat();
    let expected = Decimal::from(samples_of_both).checked_div(Decimal::from(sample_rate));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[test]
fn media_duration_adds_the_padding_bit_to_the_frame_length() {
    let padded = [0xFF, 0xFB, 0x92, 0x00];
    let bytes = [
        frame(padded, 418),
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
        frame(padded, 418),
    ]
    .concat();
    let expected = Decimal::from(3456_u32).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[test]
fn media_duration_sums_each_sample_rate_exactly() {
    let bytes = [
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
        frame([0xFF, 0xFB, 0x94, 0x00], 384),
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
    ]
    .concat();
    let at_44k = Decimal::from(2304_u32).checked_div(Decimal::from(44_100_u32));
    let at_48k = Decimal::from(1152_u32).checked_div(Decimal::from(48_000_u32));
    assert_eq!(
        mp3_seconds(Cursor::new(bytes)),
        at_44k.unwrap().checked_add(at_48k.unwrap())
    );
}

#[rstest]
#[case::xing(*b"Xing", 1, 115_200)]
#[case::info(*b"Info", 1, 115_200)]
#[case::frames_flag_among_others(*b"Xing", 3, 115_200)]
#[case::frames_flag_clear(*b"Xing", 2, 4608)]
#[case::other_tag(*b"Xinh", 1, 4608)]
fn media_duration_trusts_a_xing_frame_count_only_when_flagged(
    #[case] tag: [u8; 4],
    #[case] flags: u32,
    #[case] samples: u64,
) {
    let bytes = [
        xing_frame(STEREO_FRAME, STEREO_FRAME_BYTES, 32, tag, (flags, 100)),
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
    ]
    .concat();
    let expected = Decimal::from(samples).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[rstest]
#[case::v1_stereo([0xFF, 0xFB, 0x90, 0x00], 417, 32, 11_520, 44_100)]
#[case::v1_mono([0xFF, 0xFB, 0x90, 0xC0], 417, 17, 11_520, 44_100)]
#[case::v2_stereo([0xFF, 0xF3, 0x80, 0x00], 208, 17, 5760, 22_050)]
#[case::v2_mono([0xFF, 0xF3, 0x80, 0xC0], 208, 9, 5760, 22_050)]
fn media_duration_finds_the_xing_fields_after_the_side_information(
    #[case] header: [u8; 4],
    #[case] length: usize,
    #[case] side_information: usize,
    #[case] samples: u64,
    #[case] sample_rate: u32,
) {
    let bytes = [
        xing_frame(header, length, side_information, *b"Xing", (1, 10)),
        frame(header, length),
    ]
    .concat();
    let expected = Decimal::from(samples).checked_div(Decimal::from(sample_rate));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[rstest]
#[case::trailing_zeros(vec![0; 5])]
#[case::partial_header(vec![0xFF])]
#[case::partial_header_of_three(vec![0xFF, 0xFB, 0x90])]
fn media_duration_ignores_trailing_padding_and_a_partial_header(#[case] trailer: Vec<u8>) {
    let bytes = [frame(STEREO_FRAME, STEREO_FRAME_BYTES), trailer].concat();
    let expected = Decimal::from(1152_u32).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[test]
fn media_duration_skips_zero_bytes_before_a_frame() {
    let bytes = [vec![0; 3], frame(STEREO_FRAME, STEREO_FRAME_BYTES)].concat();
    let expected = Decimal::from(1152_u32).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[rstest]
#[case::no_footer(0x00, 0)]
#[case::footer(0x10, 10)]
#[case::other_flags(0xE0, 0)]
fn media_duration_skips_an_id3v2_tag_and_its_footer(#[case] flags: u8, #[case] footer: usize) {
    let bytes = [
        b"ID3\x04\x00".to_vec(),
        vec![flags, 0, 0, 1, 72],
        vec![0x01; 200],
        vec![0x01; footer],
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
    ]
    .concat();
    let expected = Decimal::from(1152_u32).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes.clone())), expected);
    assert_eq!(media_duration_seconds(Cursor::new(bytes)), expected);
}

#[test]
fn media_duration_skips_a_trailing_id3v1_tag() {
    let bytes = [
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
        b"TAG".to_vec(),
        vec![0x20; 125],
    ]
    .concat();
    let expected = Decimal::from(1152_u32).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[test]
fn media_duration_skips_an_ape_tag_by_its_declared_size() {
    let bytes = [
        b"APETAGEX".to_vec(),
        vec![0xD0, 0x07, 0x00, 0x00],
        100_u32.to_le_bytes().to_vec(),
        vec![0x01; 116],
        frame(STEREO_FRAME, STEREO_FRAME_BYTES),
    ]
    .concat();
    let expected = Decimal::from(1152_u32).checked_div(Decimal::from(44_100_u32));
    assert_eq!(mp3_seconds(Cursor::new(bytes)), expected);
}

#[rstest]
#[case::empty(Vec::new())]
#[case::only_zeros(vec![0; 16])]
#[case::junk_before_a_frame([b"junk".to_vec(), frame(STEREO_FRAME, STEREO_FRAME_BYTES)].concat())]
#[case::first_byte_is_not_a_sync(frame([0x41, 0xFB, 0x90, 0x00], STEREO_FRAME_BYTES))]
#[case::second_byte_is_not_a_sync(frame([0xFF, 0x1B, 0x90, 0x00], STEREO_FRAME_BYTES))]
#[case::reserved_version(frame([0xFF, 0xEF, 0x90, 0x00], STEREO_FRAME_BYTES))]
#[case::reserved_layer(frame([0xFF, 0xF9, 0x90, 0x00], STEREO_FRAME_BYTES))]
#[case::free_bit_rate(frame([0xFF, 0xFB, 0x00, 0x00], STEREO_FRAME_BYTES))]
#[case::reserved_bit_rate(frame([0xFF, 0xFB, 0xF0, 0x00], STEREO_FRAME_BYTES))]
#[case::reserved_sample_rate(frame([0xFF, 0xFB, 0x9C, 0x00], STEREO_FRAME_BYTES))]
#[case::frame_shorter_than_its_fields(frame([0xFF, 0xFF, 0x14, 0x00], 100))]
#[case::truncated_side_information(frame(STEREO_FRAME, STEREO_FRAME_BYTES)[..20].to_vec())]
#[case::truncated_xing_fields(frame(STEREO_FRAME, STEREO_FRAME_BYTES)[..40].to_vec())]
#[case::truncated_body(frame(STEREO_FRAME, STEREO_FRAME_BYTES)[..200].to_vec())]
#[case::xing_declaring_no_frames(xing_frame(STEREO_FRAME, STEREO_FRAME_BYTES, 32, *b"Xing", (1, 0)))]
#[case::truncated_id3v2_header(b"ID3\x04\x00".to_vec())]
#[case::id3v2_size_past_the_end([b"ID3\x04\x00".to_vec(), vec![0, 0, 0, 1, 72], vec![0x01; 10]].concat())]
#[case::truncated_id3v1([frame(STEREO_FRAME, STEREO_FRAME_BYTES), b"TAG".to_vec(), vec![0x20; 20]].concat())]
#[case::ape_without_its_magic([b"APETxxxxxxxxxxxx".to_vec(), vec![0x01; 200]].concat())]
fn media_duration_is_none_for_an_unmeasurable_mp3_stream(#[case] bytes: Vec<u8>) {
    assert_eq!(mp3_seconds(Cursor::new(bytes)), None);
}

#[rstest]
#[case::at_the_header(vec![])]
#[case::in_the_zero_padding(vec![0])]
#[case::in_the_header(vec![0xFF])]
fn media_duration_is_none_when_the_reader_fails_after_a_frame(#[case] trailer: Vec<u8>) {
    let data = [frame(STEREO_FRAME, STEREO_FRAME_BYTES), trailer].concat();
    let source = FailingAfter {
        data: Cursor::new(data),
    };
    assert_eq!(mp3_seconds(source), None);
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
