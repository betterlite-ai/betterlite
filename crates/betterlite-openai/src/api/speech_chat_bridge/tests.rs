#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::iter::repeat_n;
use std::path::Path;
use std::str::from_utf8;

use super::audio_base64::decode_chat_audio_base64;
use super::media_type::{resolve_speech_media_type, sniff_speech_audio};
use crate::api::audio_file_input::decoder::AudioDecodeLimits;

// an OggS page whose OpusHead identifier starts `prefix_len` bytes after the capture pattern
fn ogg_page(prefix_len: usize) -> Vec<u8> {
    let mut audio = b"OggS".to_vec();
    audio.extend(repeat_n(0_u8, prefix_len));
    audio.extend_from_slice(b"OpusHead");
    audio
}

fn sniffed(audio: &[u8]) -> Option<&'static str> {
    sniff_speech_audio(audio, &AudioDecodeLimits::default())
}

fn decoded(data: &str) -> Vec<u8> {
    decode_chat_audio_base64(data).expect("the base64 audio")
}

#[test]
fn speech_media_precedence() {
    assert_eq!(
        resolve_speech_media_type(Some(" AUDIO/FLAC; charset=x "), Some("mp3")),
        "audio/flac"
    );
    assert_eq!(
        resolve_speech_media_type(Some("application/octet-stream"), Some("WAV")),
        "audio/wav"
    );
    assert_eq!(
        resolve_speech_media_type(Some("application/octet-stream"), None),
        "audio/mpeg"
    );
    assert_eq!(
        resolve_speech_media_type(Some("application/octet-stream"), Some("mpeg")),
        "audio/mpeg"
    );
    assert_eq!(resolve_speech_media_type(None, Some("ogg")), "audio/mpeg");
    assert_eq!(
        resolve_speech_media_type(Some("audio/custom"), None),
        "audio/custom"
    );
    assert_eq!(sniffed(b"RIFF\x00\x00\x00\x00WAVE"), Some("audio/wav"));
    assert_eq!(sniffed(b"fLaC"), Some("audio/flac"));
    assert_eq!(sniffed(b"ID3"), Some("audio/mpeg"));
    assert_eq!(sniffed(&ogg_page(20)), Some("audio/opus"));
    assert_eq!(sniffed(&ogg_page(52)), Some("audio/opus"));
    assert_eq!(sniffed(&ogg_page(53)), Some("audio/ogg"));
}

#[test]
fn speech_media_frame_guards() {
    assert_eq!(sniffed(&[0xFF, 0xF1, 0x50]), Some("audio/aac"));
    assert_eq!(sniffed(&[0xFF, 0x7B, 0x90]), None);
    assert_eq!(sniffed(&[0xFF, 0xF1, 0x34]), None);
    assert_eq!(sniffed(&[0xFF, 0xF1, 0x3C]), None);
    assert_eq!(sniffed(&[0xFF, 0xF9, 0x90]), Some("audio/aac"));
    assert_eq!(sniffed(&[0xFF, 0xFB, 0x90]), Some("audio/mpeg"));
    assert_eq!(sniffed(&[0xFF, 0xEB, 0x90]), None);
    assert_eq!(sniffed(&[0xFF, 0xE1, 0x90]), None);
    assert_eq!(sniffed(&[0xFF, 0xFB, 0xF0]), None);
    assert_eq!(sniffed(&[0xFF, 0xFB, 0x9C]), None);
    assert_eq!(sniffed(&[0xFF, 0xF1]), None);
    assert_eq!(sniffed(&[0x00, 0xF1, 0x50]), None);
    assert_eq!(sniffed(b"hello"), None);
    assert_eq!(sniffed(b""), None);
}

#[test]
fn decodes_chat_audio_base64() {
    assert_eq!(decoded(""), b"");
    assert_eq!(decoded("QUJD"), b"ABC");
    assert_eq!(decoded("QUJD="), b"ABC");
    assert_eq!(decoded("YQ=="), b"a");
    assert_eq!(decoded("Y Q==!"), b"a");
    assert_eq!(decoded("ABCD"), [0x00, 0x10, 0x83]);
    assert_eq!(decoded("QR=="), [0x41]);
    assert_eq!(decoded("aGVsbG8="), b"hello");
    assert_eq!(decoded("MDEy"), b"012");
    assert_eq!(decoded("+w=="), [0xFB]);
    assert_eq!(decoded("/w=="), [0xFF]);
}

#[test]
fn decodes_chat_audio_pad_runs() {
    assert_eq!(decoded("QQ==QQ=="), [0x41]);
    assert_eq!(decoded("ab==a"), [0x69]);
    assert_eq!(decoded("QQ=Q="), [0x41, 0x04]);
    assert_eq!(decoded("=QUJD"), b"ABC");
    assert_eq!(decoded("===="), b"");
}

#[test]
fn sniff_media_type_honors_the_byte_limit() {
    let body = b"RIFF\x00\x00\x00\x00WAVE";
    let exact = AudioDecodeLimits {
        max_bytes: body.len(),
        max_frames: 33_177_600_000,
        max_sample_rate: 384_000,
    };
    assert_eq!(sniff_speech_audio(body, &exact), Some("audio/wav"));
    let short = AudioDecodeLimits {
        max_bytes: body.len().saturating_sub(1),
        max_frames: 33_177_600_000,
        max_sample_rate: 384_000,
    };
    assert_eq!(sniff_speech_audio(body, &short), None);
    assert_eq!(sniffed(b"RIFF"), None);
}

#[test]
fn rejects_bad_base64() {
    let length = "Invalid base64-encoded string: number of data characters (1) cannot be 1 more than a multiple of 4";
    let lone = decode_chat_audio_base64("Q").expect_err("a lone character");
    assert_eq!(lone.to_string(), length);
    let padded_lone = decode_chat_audio_base64("Q===").expect_err("a lone character");
    assert_eq!(padded_lone.to_string(), length);
    let two_missing = decode_chat_audio_base64("QQ").expect_err("two pads missing");
    assert_eq!(two_missing.to_string(), "Incorrect padding");
    let one_missing = decode_chat_audio_base64("QQ=").expect_err("one pad missing");
    assert_eq!(one_missing.to_string(), "Incorrect padding");
    let three_chars = decode_chat_audio_base64("QUI").expect_err("one pad missing");
    assert_eq!(three_chars.to_string(), "Incorrect padding");
    let interrupted = decode_chat_audio_base64("Q=Q=").expect_err("one pad short");
    assert_eq!(interrupted.to_string(), "Incorrect padding");
    let unpadded = decode_chat_audio_base64("QR").expect_err("two pads missing");
    assert_eq!(unpadded.to_string(), "Incorrect padding");
    let non_ascii = decode_chat_audio_base64("\u{e9}").expect_err("non-ascii");
    assert_eq!(
        non_ascii.to_string(),
        "string argument should contain only ASCII characters"
    );
}

// one named outcome per corpus seed, so a changed decoder or sniffer fails the named case
const EXPECTED: &[(&str, &str)] = &[
    ("base64-empty", "base64:ok:[]"),
    ("base64-quantum", "base64:ok:[41, 42, 43]"),
    ("base64-padded", "base64:ok:[61]"),
    ("base64-garbage-skipped", "base64:ok:[61]"),
    ("base64-pads-reset", "base64:ok:[41]"),
    (
        "base64-length-error",
        "base64:err:Invalid base64-encoded string: number of data characters (1) cannot be 1 more than a multiple of 4",
    ),
    ("base64-padding-error", "base64:err:Incorrect padding"),
    (
        "base64-non-ascii",
        "base64:err:string argument should contain only ASCII characters",
    ),
    ("base64-not-utf8", "base64:utf8"),
    ("sniff-riff-wave", "sniff:audio/wav"),
    ("sniff-flac", "sniff:audio/flac"),
    ("sniff-ogg-opus", "sniff:audio/opus"),
    ("sniff-opus-head-at-56", "sniff:audio/opus"),
    ("sniff-opus-head-at-57", "sniff:audio/ogg"),
    ("sniff-id3", "sniff:audio/mpeg"),
    ("sniff-adts-aac", "sniff:audio/aac"),
    ("sniff-adts-rate-13", "sniff:none"),
    ("sniff-mpeg", "sniff:audio/mpeg"),
    ("sniff-mpeg-reserved-version", "sniff:none"),
    ("sniff-short", "sniff:none"),
    ("sniff-not-audio", "sniff:none"),
];

fn outcome(bytes: &[u8]) -> String {
    let Some((&selector, input)) = bytes.split_first() else {
        return String::from("empty");
    };
    if selector == 0 {
        let Ok(text) = from_utf8(input) else {
            return String::from("base64:utf8");
        };
        return match decode_chat_audio_base64(text) {
            Ok(audio) => format!("base64:ok:{audio:02x?}"),
            Err(error) => format!("base64:err:{error}"),
        };
    }
    sniffed(input).map_or_else(
        || String::from("sniff:none"),
        |media_type| format!("sniff:{media_type}"),
    )
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/speech_audio");
    let observed: BTreeMap<String, String> = fs::read_dir(corpus)
        .expect("the speech audio corpus")
        .map(|entry| {
            let path = entry.expect("a corpus entry").path();
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("a corpus name");
            let bytes = fs::read(&path).expect("a corpus seed");
            (name.to_owned(), outcome(&bytes))
        })
        .collect();
    let expected: BTreeMap<String, String> = EXPECTED
        .iter()
        .map(|(name, result)| ((*name).to_owned(), (*result).to_owned()))
        .collect();
    assert_eq!(observed, expected);
}
