#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use rstest::rstest;
use rust_decimal::Decimal;

use super::decoder::{AudioDecodeLimits, AudioDecoder, NativeAudioDecoder, decode_audio_duration};
use super::duration::calculate_request_duration;
use super::hashing::get_audio_file_content_hash;
use super::language::normalize_language_to_bcp47;
use super::{
    AudioFile, AudioFileError, AudioStream, AudioTuple, AuthorizedAudioFiles, HEALTH_CHECK_AUDIO,
    TupleContent, get_audio_file_name, open_health_check_audio, process_audio_file,
};

const ABC_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const VOICE_NAME_HASH: &str = "185e76072c429f1276e6fcd31049aef32e67d4af396da5d22e96b7fa767d403a";
const MISSING_VOICE_HASH: &str = "6285526108c0e73296b9e3ec740302004a6ea24a609ee6d7914bbd9fa2468c1d";
const TUPLE_IDENTITY_HASH: &str =
    "635c5c0e63768ed4c0b1f79e5fdb932d74425aedc4758c7c07e66fd24aa6b7c2";
const C_HASH: &str = "2e7d2c03a9507ae265ecf5b5356885a53393a2029d241394997265a1a25aefc6";

struct Files(BTreeMap<PathBuf, Vec<u8>>);

impl AuthorizedAudioFiles for Files {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(io::Cursor::new(self.read(path)?)))
    }
}

struct NoFiles {
    opens: Cell<usize>,
}

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        self.opens.set(self.opens.get().saturating_add(1));
        Err(io::Error::other(
            "a value that is not a path must not open a file",
        ))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        self.opens.set(self.opens.get().saturating_add(1));
        Err(io::Error::other(
            "a value that is not a path must not open a file",
        ))
    }
}

fn files(entries: &[(&str, &[u8])]) -> Files {
    Files(
        entries
            .iter()
            .map(|(path, content)| (PathBuf::from(path), (*content).to_vec()))
            .collect(),
    )
}

fn no_files() -> NoFiles {
    NoFiles {
        opens: Cell::new(0),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flaw {
    Sound,
    ReadFails,
    ReadReturnsNone,
    SeekFails,
    SeekMissing,
    TellMissing,
}

struct Stream {
    name: Option<String>,
    content: Vec<u8>,
    position: usize,
    flaw: Flaw,
}

impl Stream {
    fn named(name: &str, content: &[u8]) -> Self {
        Self {
            name: Some(name.to_owned()),
            content: content.to_vec(),
            position: 0,
            flaw: Flaw::Sound,
        }
    }

    fn anonymous(content: &[u8]) -> Self {
        Self {
            name: None,
            content: content.to_vec(),
            position: 0,
            flaw: Flaw::Sound,
        }
    }

    fn at(mut self, position: usize) -> Self {
        self.position = position;
        self
    }

    fn with(mut self, flaw: Flaw) -> Self {
        self.flaw = flaw;
        self
    }
}

impl AudioStream for Stream {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.flaw == Flaw::ReadFails {
            return Err(io::Error::other("read failed"));
        }
        if self.flaw == Flaw::ReadReturnsNone {
            return Ok(None);
        }
        let start = self.position.min(self.content.len());
        let content = self.content.get(start..).map(<[u8]>::to_vec);
        self.position = self.content.len();
        Ok(content)
    }

    fn tell(&mut self) -> Option<io::Result<u64>> {
        (self.flaw != Flaw::TellMissing).then(|| Ok(u64::try_from(self.position).unwrap()))
    }

    fn seek(&mut self, position: u64) -> Option<io::Result<()>> {
        let outcome = match self.flaw {
            Flaw::SeekMissing => return None,
            Flaw::SeekFails => Err(io::Error::other("seek failed")),
            Flaw::Sound | Flaw::ReadFails | Flaw::ReadReturnsNone | Flaw::TellMissing => Ok(()),
        };
        if outcome.is_ok() {
            self.position = usize::try_from(position).unwrap();
        }
        Some(outcome)
    }
}

fn wav_bytes(frames: u32, sample_rate: u32, channels: u16) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    for _ in 0..frames {
        for _ in 0..channels {
            writer.write_sample(0_i16).unwrap();
        }
    }
    writer.finalize().unwrap();
    cursor.into_inner()
}

fn limits(max_bytes: usize, max_frames: u64, max_sample_rate: u32) -> AudioDecodeLimits {
    AudioDecodeLimits {
        max_bytes,
        max_frames,
        max_sample_rate,
    }
}

fn native_limits() -> AudioDecodeLimits {
    AudioDecodeLimits::default()
}

fn duration(bytes: &[u8]) -> Option<Decimal> {
    let decoder = NativeAudioDecoder {
        limits: native_limits(),
    };
    calculate_request_duration(
        &mut AudioFile::Bytes(bytes),
        &no_files(),
        AudioDecoder::Available(&decoder),
    )
}

fn duration_with(
    file: &mut AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
    limits: AudioDecodeLimits,
) -> Option<Decimal> {
    let decoder = NativeAudioDecoder { limits };
    calculate_request_duration(file, files, AudioDecoder::Available(&decoder))
}

fn hash_of(file: &mut AudioFile<'_>) -> String {
    get_audio_file_content_hash(file, &no_files())
}

#[rstest]
#[case::bytes(AudioFile::Bytes(b"abc"), "audio.wav", "audio/wav", b"abc")]
#[case::byte_array(AudioFile::ByteArray(b"abc"), "audio.wav", "audio/wav", b"abc")]
#[case::empty_bytes(AudioFile::Bytes(b""), "audio.wav", "audio/wav", b"")]
#[case::mp3_tuple(
    AudioFile::Tuple(AudioTuple {
        filename: "voice.MP3",
        content: Some(TupleContent::Bytes(b"abc")),
    }),
    "voice.MP3",
    "audio/mpeg",
    b"abc"
)]
#[case::empty_tuple_name(
    AudioFile::Tuple(AudioTuple {
        filename: "",
        content: Some(TupleContent::Bytes(b"abc")),
    }),
    "audio.wav",
    "audio/wav",
    b"abc"
)]
#[case::missing_tuple_name(
    AudioFile::Tuple(AudioTuple {
        filename: "",
        content: Some(TupleContent::Bytes(b"abc")),
    }),
    "audio.wav",
    "audio/wav",
    b"abc"
)]
#[case::tuple_without_extension(
    AudioFile::Tuple(AudioTuple {
        filename: "voice",
        content: Some(TupleContent::ByteArray(b"abc")),
    }),
    "voice",
    "audio/wav",
    b"abc"
)]
#[case::tuple_unknown_extension(
    AudioFile::Tuple(AudioTuple {
        filename: "voice.custom",
        content: Some(TupleContent::Bytes(b"abc")),
    }),
    "voice.custom",
    "audio/wav",
    b"abc"
)]
#[case::tuple_last_extension(
    AudioFile::Tuple(AudioTuple {
        filename: "archive.tar.wav",
        content: Some(TupleContent::Bytes(b"abc")),
    }),
    "archive.tar.wav",
    "audio/wav",
    b"abc"
)]
#[case::tuple_trailing_dot(
    AudioFile::Tuple(AudioTuple {
        filename: "voice.",
        content: Some(TupleContent::Bytes(b"abc")),
    }),
    "voice.",
    "audio/wav",
    b"abc"
)]
fn audio_input_shapes(
    #[case] file: AudioFile<'static>,
    #[case] filename: &str,
    #[case] content_type: &str,
    #[case] content: &[u8],
) {
    let processed = process_audio_file(file, &no_files()).unwrap();
    assert_eq!(processed.filename, filename);
    assert_eq!(processed.content_type, content_type);
    assert_eq!(processed.file_content, content);
}

#[test]
fn streams_read_from_the_current_position_and_rewind() {
    let mut stream = Stream::named("voice.wav", b"abc").at(1);
    let processed = process_audio_file(AudioFile::Stream(&mut stream), &no_files()).unwrap();
    assert_eq!(processed.file_content, b"bc");
    assert_eq!(processed.filename, "voice.wav");
    assert_eq!(processed.content_type, "audio/wav");
    assert_eq!(stream.position, 0);
}

#[test]
fn tuple_streams_follow_the_stream_rule() {
    let mut stream = Stream::named("voice.wav", b"abc").at(2);
    let file = AudioFile::Tuple(AudioTuple {
        filename: "other.mp3",
        content: Some(TupleContent::Stream(&mut stream)),
    });
    let processed = process_audio_file(file, &no_files()).unwrap();
    assert_eq!(processed.file_content, b"c");
    assert_eq!(processed.filename, "other.mp3");
    assert_eq!(processed.content_type, "audio/mpeg");
    assert_eq!(stream.position, 0);
}

#[test]
fn anonymous_streams_use_the_default_name() {
    let mut stream = Stream::anonymous(b"abc");
    let processed = process_audio_file(AudioFile::Stream(&mut stream), &no_files()).unwrap();
    assert_eq!(processed.filename, "audio.wav");
    assert_eq!(processed.content_type, "audio/wav");
}

#[test]
fn authorized_paths_use_the_final_component() {
    let fixtures = files(&[("/authorized/audio/clip.wav", b"abc")]);
    let processed = process_audio_file(
        AudioFile::Path(Path::new("/authorized/audio/clip.wav")),
        &fixtures,
    )
    .unwrap();
    assert_eq!(processed.filename, "clip.wav");
    assert_eq!(processed.content_type, "audio/wav");
    assert_eq!(processed.file_content, b"abc");
}

#[test]
fn a_path_with_a_trailing_slash_has_no_name() {
    let fixtures = files(&[("/authorized/audio/", b"abc")]);
    let processed =
        process_audio_file(AudioFile::Path(Path::new("/authorized/audio/")), &fixtures).unwrap();
    assert_eq!(processed.filename, "");
    assert_eq!(processed.content_type, "audio/wav");
}

#[test]
fn audio_input_rejections() {
    let files = no_files();
    assert_eq!(
        process_audio_file(AudioFile::Text("voice.wav"), &files)
            .unwrap_err()
            .to_string(),
        "process_audio_file does not accept bare str inputs. Pass bytes, an open file handle, a (filename, content) tuple, or a pathlib.Path."
    );
    assert_eq!(
        process_audio_file(
            AudioFile::Tuple(AudioTuple {
                filename: "voice.wav",
                content: Some(TupleContent::Text("abc")),
            }),
            &files
        )
        .unwrap_err()
        .to_string(),
        "process_audio_file does not accept bare str tuple contents. Pass bytes, an open file handle, or a pathlib.Path."
    );
}

#[test]
fn audio_input_rejections_name_the_unsupported_type() {
    let files = no_files();
    assert_eq!(
        process_audio_file(
            AudioFile::Tuple(AudioTuple {
                filename: "voice.wav",
                content: None,
            }),
            &files
        )
        .unwrap_err()
        .to_string(),
        "Tuple must have at least 2 elements: (filename, content)"
    );
    assert_eq!(
        process_audio_file(
            AudioFile::Tuple(AudioTuple {
                filename: "voice.wav",
                content: Some(TupleContent::Unsupported {
                    python_class: "int"
                }),
            }),
            &files
        )
        .unwrap_err()
        .to_string(),
        "Unsupported content type in tuple: <class 'int'>"
    );
    assert_eq!(
        process_audio_file(
            AudioFile::Unsupported {
                python_class: "int"
            },
            &files
        )
        .unwrap_err()
        .to_string(),
        "Unsupported audio_file type: <class 'int'>"
    );
}

#[test]
fn a_reader_returning_none_has_no_content_and_is_rewound() {
    let mut stream = Stream::named("voice.wav", b"abc")
        .at(1)
        .with(Flaw::ReadReturnsNone);
    let error = process_audio_file(AudioFile::Stream(&mut stream), &no_files()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not extract file content from audio_file"
    );
    assert_eq!(stream.position, 0);
}

#[test]
fn read_open_and_seek_failures_propagate() {
    let mut failing_read = Stream::named("voice.wav", b"abc").with(Flaw::ReadFails);
    assert!(matches!(
        process_audio_file(AudioFile::Stream(&mut failing_read), &no_files()),
        Err(AudioFileError::Read { .. })
    ));
    let mut failing_seek = Stream::named("voice.wav", b"abc").with(Flaw::SeekFails);
    assert!(matches!(
        process_audio_file(AudioFile::Stream(&mut failing_seek), &no_files()),
        Err(AudioFileError::Seek { .. })
    ));
    assert!(matches!(
        process_audio_file(AudioFile::Path(Path::new("/missing.wav")), &no_files()),
        Err(AudioFileError::Open { .. })
    ));
    assert!(matches!(
        process_audio_file(
            AudioFile::Tuple(AudioTuple {
                filename: "voice.wav",
                content: Some(TupleContent::Path(Path::new("/missing.wav"))),
            }),
            &no_files()
        ),
        Err(AudioFileError::Open { .. })
    ));
}

#[test]
fn audio_hash_and_position() {
    assert_eq!(hash_of(&mut AudioFile::Bytes(b"abc")), ABC_HASH);
    assert_eq!(hash_of(&mut AudioFile::ByteArray(b"abc")), ABC_HASH);
    let mut tuple = AudioFile::Tuple(AudioTuple {
        filename: "different",
        content: Some(TupleContent::Bytes(b"abc")),
    });
    assert_eq!(hash_of(&mut tuple), ABC_HASH);
    let mut stream = Stream::named("voice.wav", b"abc");
    assert_eq!(hash_of(&mut AudioFile::Stream(&mut stream)), ABC_HASH);
    assert_eq!(stream.position, 0);
}

#[test]
fn hashing_a_stream_restores_its_position() {
    let mut stream = Stream::named("voice.wav", b"abc").at(2);
    assert_eq!(hash_of(&mut AudioFile::Stream(&mut stream)), ABC_HASH);
    assert_eq!(stream.position, 2);
}

#[test]
fn hashing_a_string_opens_no_file() {
    let files = no_files();
    let mut text = AudioFile::Text("abc");
    assert_eq!(get_audio_file_content_hash(&mut text, &files), ABC_HASH);
    let mut path_text = AudioFile::Text("/no/such/audio.wav");
    assert_eq!(
        get_audio_file_content_hash(&mut path_text, &files),
        "d2bf1f2aa8cab96dee89009826f49db6b5e2cd4e28c10e5ef15dd2f87d7867fd"
    );
    assert_eq!(files.opens.get(), 0);
}

#[test]
fn hash_fallbacks_use_the_reader_name_and_the_path() {
    let mut stream = Stream::named("voice.wav", b"abc").with(Flaw::ReadFails);
    assert_eq!(
        get_audio_file_content_hash(&mut AudioFile::Stream(&mut stream), &no_files()),
        VOICE_NAME_HASH
    );
    let mut path = AudioFile::Path(Path::new("/missing/voice.wav"));
    assert_eq!(
        get_audio_file_content_hash(&mut path, &no_files()),
        MISSING_VOICE_HASH
    );
}

#[test]
fn a_missing_or_empty_tuple_name_uses_the_tuple_identity() {
    let mut failing = Stream::named("voice.wav", b"abc").with(Flaw::ReadFails);
    let mut empty = AudioFile::Tuple(AudioTuple {
        filename: "",
        content: Some(TupleContent::Stream(&mut failing)),
    });
    assert_eq!(
        get_audio_file_content_hash(&mut empty, &no_files()),
        TUPLE_IDENTITY_HASH
    );
    let mut missing = AudioFile::Tuple(AudioTuple {
        filename: "",
        content: Some(TupleContent::Stream(&mut failing)),
    });
    assert_eq!(
        get_audio_file_content_hash(&mut missing, &no_files()),
        TUPLE_IDENTITY_HASH
    );
}

#[test]
fn a_short_tuple_hashes_its_filename() {
    let mut short = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: None,
    });
    assert_eq!(
        get_audio_file_content_hash(&mut short, &no_files()),
        VOICE_NAME_HASH
    );
}

#[test]
fn a_tuple_string_content_overrides_its_filename() {
    let mut file = AudioFile::Tuple(AudioTuple {
        filename: "other.mp3",
        content: Some(TupleContent::Text("abc")),
    });
    assert_eq!(hash_of(&mut file), ABC_HASH);
}

#[test]
fn a_tuple_path_content_overrides_its_filename_when_the_open_fails() {
    let mut file = AudioFile::Tuple(AudioTuple {
        filename: "other.mp3",
        content: Some(TupleContent::Path(Path::new("/missing/voice.wav"))),
    });
    assert_eq!(hash_of(&mut file), MISSING_VOICE_HASH);
}

#[test]
fn hashing_a_stream_without_seek_reads_from_its_position() {
    let mut stream = Stream::named("voice.wav", b"abc")
        .at(2)
        .with(Flaw::SeekMissing);
    assert_eq!(hash_of(&mut AudioFile::Stream(&mut stream)), C_HASH);
    assert_eq!(stream.position, 3);
}

#[test]
fn hashing_a_stream_without_tell_does_not_restore() {
    let mut stream = Stream::named("voice.wav", b"abc")
        .at(2)
        .with(Flaw::TellMissing);
    assert_eq!(hash_of(&mut AudioFile::Stream(&mut stream)), ABC_HASH);
    assert_eq!(stream.position, 3);
}

#[test]
fn a_failed_seek_falls_back_to_the_reader_name() {
    let mut stream = Stream::named("voice.wav", b"abc").with(Flaw::SeekFails);
    assert_eq!(
        hash_of(&mut AudioFile::Stream(&mut stream)),
        VOICE_NAME_HASH
    );
}

#[test]
fn get_audio_file_name_prefers_the_name() {
    let mut stream = Stream::named("voice.wav", b"abc");
    assert_eq!(
        get_audio_file_name(&AudioFile::Stream(&mut stream)),
        "voice.wav"
    );
    let mut anonymous = Stream::anonymous(b"abc");
    assert_eq!(
        get_audio_file_name(&AudioFile::Stream(&mut anonymous)),
        "audio.wav"
    );
    assert_eq!(
        get_audio_file_name(&AudioFile::Path(Path::new("/a/b/voice.wav"))),
        "/a/b/voice.wav"
    );
    assert_eq!(get_audio_file_name(&AudioFile::Text("abc")), "abc");
    assert_eq!(get_audio_file_name(&AudioFile::Bytes(b"abc")), "audio.wav");
    let tuple = AudioTuple {
        filename: "voice.wav",
        content: Some(TupleContent::Bytes(b"abc")),
    };
    assert_eq!(
        get_audio_file_name(&AudioFile::Tuple(tuple)),
        "(voice.wav, bytes)"
    );
}

#[rstest]
#[case::english("en", "en-US")]
#[case::spanish("es", "es-ES")]
#[case::german("de", "de-DE")]
#[case::french("fr", "fr-FR")]
#[case::italian("it", "it-IT")]
#[case::portuguese("pt", "pt-BR")]
#[case::japanese("ja", "ja-JP")]
#[case::korean("ko", "ko-KR")]
#[case::chinese("zh", "zh-CN")]
#[case::russian("ru", "ru-RU")]
#[case::hindi("hi", "hi-IN")]
#[case::arabic("ar", "ar-SA")]
#[case::uppercase("EN", "en-US")]
#[case::qualified("en-gb", "en-gb")]
#[case::unknown("zz", "zz")]
#[case::padded(" en ", " en ")]
fn transcription_language_codes(#[case] language: &str, #[case] expected: &str) {
    assert_eq!(normalize_language_to_bcp47(language), expected);
}

#[test]
fn audio_duration_and_health() {
    let mono = wav_bytes(16_000, 16_000, 1);
    let stereo = wav_bytes(16_000, 16_000, 2);
    assert_eq!(duration(&mono), Some(Decimal::ONE));
    assert_eq!(duration(&stereo), Some(Decimal::ONE));
    assert_eq!(duration(&wav_bytes(0, 16_000, 1)), Some(Decimal::ZERO));
    let mut handle = open_health_check_audio(
        &files(&[("/health/audio_health_check.wav", HEALTH_CHECK_AUDIO)]),
        Path::new("/health"),
    )
    .unwrap();
    let mut content = Vec::new();
    handle.read_to_end(&mut content).unwrap();
    assert_eq!(content, HEALTH_CHECK_AUDIO);
    assert!(!content.is_empty());
}

#[test]
fn duration_accepts_every_supported_shape() {
    let clip = wav_bytes(1_000, 1_000, 1);
    let fixtures = files(&[("/authorized/clip.wav", &clip)]);
    let mut path = AudioFile::Path(Path::new("/authorized/clip.wav"));
    assert_eq!(
        duration_with(&mut path, &fixtures, native_limits()),
        Some(Decimal::ONE)
    );
    let mut stream = Stream::named("voice.wav", &clip).at(2);
    assert_eq!(
        duration_with(
            &mut AudioFile::Stream(&mut stream),
            &fixtures,
            native_limits()
        ),
        Some(Decimal::ONE)
    );
    assert_eq!(stream.position, 2);
    let mut tuple_bytes = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: Some(TupleContent::Bytes(&clip)),
    });
    assert_eq!(
        duration_with(&mut tuple_bytes, &fixtures, native_limits()),
        Some(Decimal::ONE)
    );
    let mut tuple_stream = Stream::named("voice.wav", &clip).at(1);
    let mut tuple = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: Some(TupleContent::Stream(&mut tuple_stream)),
    });
    assert_eq!(
        duration_with(&mut tuple, &fixtures, native_limits()),
        Some(Decimal::ONE)
    );
    assert_eq!(tuple_stream.position, 1);
}

#[test]
fn duration_returns_none_without_decodable_content() {
    let clip = wav_bytes(1_000, 1_000, 1);
    let mut text = AudioFile::Text("voice.wav");
    assert_eq!(duration_with(&mut text, &no_files(), native_limits()), None);
    let mut malformed = AudioFile::Bytes(b"not audio");
    assert_eq!(
        duration_with(&mut malformed, &no_files(), native_limits()),
        None
    );
    let mut unavailable = AudioFile::Bytes(&clip);
    assert_eq!(
        calculate_request_duration(&mut unavailable, &no_files(), AudioDecoder::Unavailable),
        None
    );
    let mut missing_path = AudioFile::Path(Path::new("/missing.wav"));
    assert_eq!(
        duration_with(&mut missing_path, &no_files(), native_limits()),
        None
    );
    let mut unsupported = AudioFile::Unsupported {
        python_class: "int",
    };
    assert_eq!(
        duration_with(&mut unsupported, &no_files(), native_limits()),
        None
    );
}

#[test]
fn duration_returns_none_for_tuple_shapes_it_does_not_read() {
    let clip = wav_bytes(1_000, 1_000, 1);
    let mut tuple_byte_array = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: Some(TupleContent::ByteArray(&clip)),
    });
    assert_eq!(
        duration_with(&mut tuple_byte_array, &no_files(), native_limits()),
        None
    );
    let mut tuple_path = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: Some(TupleContent::Path(Path::new("/authorized/clip.wav"))),
    });
    assert_eq!(
        duration_with(
            &mut tuple_path,
            &files(&[("/authorized/clip.wav", &clip)]),
            native_limits()
        ),
        None
    );
    let mut short_tuple = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: None,
    });
    assert_eq!(
        duration_with(&mut short_tuple, &no_files(), native_limits()),
        None
    );
    let mut tuple_text = AudioFile::Tuple(AudioTuple {
        filename: "voice.wav",
        content: Some(TupleContent::Text("abc")),
    });
    assert_eq!(
        duration_with(&mut tuple_text, &no_files(), native_limits()),
        None
    );
}

#[test]
fn duration_returns_none_when_the_reader_fails() {
    let mut stream = Stream::named("voice.wav", b"abc").with(Flaw::ReadFails);
    assert_eq!(
        duration_with(
            &mut AudioFile::Stream(&mut stream),
            &no_files(),
            native_limits()
        ),
        None
    );
    assert_eq!(stream.position, 0);
    let mut returns_none = Stream::named("voice.wav", b"abc").with(Flaw::ReadReturnsNone);
    assert_eq!(
        duration_with(
            &mut AudioFile::Stream(&mut returns_none),
            &no_files(),
            native_limits()
        ),
        None
    );
    let mut seek_fails = Stream::named("voice.wav", b"abc").with(Flaw::SeekFails);
    assert_eq!(
        duration_with(
            &mut AudioFile::Stream(&mut seek_fails),
            &no_files(),
            native_limits()
        ),
        None
    );
    let mut seek_missing = Stream::named("voice.wav", b"abc")
        .at(1)
        .with(Flaw::SeekMissing);
    assert_eq!(
        duration_with(
            &mut AudioFile::Stream(&mut seek_missing),
            &no_files(),
            native_limits()
        ),
        None
    );
}

#[test]
fn health_audio_reports_a_missing_resource() {
    assert!(matches!(
        open_health_check_audio(&no_files(), Path::new("/missing")),
        Err(AudioFileError::Open { .. })
    ));
}

#[test]
fn decode_limits_reject_oversized_audio() {
    let clip = wav_bytes(10, 10, 1);
    assert_eq!(clip.len(), 64);
    assert_eq!(
        decode_audio_duration(&clip, &limits(64, 10, 10)),
        Some(Decimal::ONE)
    );
    assert_eq!(decode_audio_duration(&clip, &limits(63, 10, 10)), None);
}

#[test]
fn decode_limits_reject_more_frames_and_faster_rates() {
    let clip = wav_bytes(10, 10, 1);
    assert_eq!(
        decode_audio_duration(&clip, &limits(64, 10, 10)),
        Some(Decimal::ONE)
    );
    assert_eq!(decode_audio_duration(&clip, &limits(64, 9, 10)), None);
    assert_eq!(decode_audio_duration(&clip, &limits(64, 10, 9)), None);
}

#[rstest]
#[case::mono_cut_mid_frame(wav_bytes(10, 10, 1), 51, Decimal::new(3, 1))]
#[case::three_channels(wav_bytes(10, 10, 3), 57, Decimal::new(2, 1))]
#[case::header_only(wav_bytes(10, 10, 1), 44, Decimal::ZERO)]
#[case::complete(wav_bytes(10, 10, 3), 104, Decimal::ONE)]
fn decode_caps_frames_at_the_bytes_present(
    #[case] clip: Vec<u8>,
    #[case] length: usize,
    #[case] expected: Decimal,
) {
    assert_eq!(
        decode_audio_duration(&clip[..length], &native_limits()),
        Some(expected)
    );
}

#[test]
fn decode_ignores_a_data_length_beyond_the_upload() {
    let mut clip = wav_bytes(2, 10, 1);
    clip[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode_audio_duration(&clip, &native_limits()),
        Some(Decimal::new(2, 1))
    );
}

#[rstest]
#[case::adpcm(2)]
#[case::a_law(6)]
#[case::mu_law(7)]
fn decode_returns_none_for_wav_codecs_it_does_not_read(#[case] format_tag: u8) {
    let mut clip = wav_bytes(10, 10, 1);
    clip[20] = format_tag;
    assert_eq!(decode_audio_duration(&clip, &native_limits()), None);
}

#[test]
fn decode_rejects_audio_that_is_not_wave() {
    assert_eq!(
        decode_audio_duration(b"not a wave file", &native_limits()),
        None
    );
    let clip = wav_bytes(10, 10, 1);
    let truncated = &clip[..20];
    assert_eq!(decode_audio_duration(truncated, &native_limits()), None);
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/audio_duration");
    let limits = native_limits();
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(seeds.len() >= 4);
    seeds
        .iter()
        .map(|path| fs::read(path).unwrap())
        .filter_map(|bytes| decode_audio_duration(&bytes, &limits))
        .for_each(drop);
}
