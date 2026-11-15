use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

use super::support::{AUDIO, audio_start, capture, new_input, send, try_send, wire};
use crate::realtime_input::error::InputError;
use crate::realtime_input::frames::{Event, EventFrame, InputFrame};
use crate::realtime_session::records::Setting;

const RESPONSE_CREATE: &str = r#"{"type":"response.create"}"#;
const APPEND: &str = r#"{"type":"input_audio_buffer.append","audio":"QUJD"}"#;
const SESSION_RATE_8000: &str =
    r#"{"type":"session.update","session":{"input_sample_rate_hertz":8000}}"#;
const FIXTURE: &[u8] = include_bytes!("../ready_trigger.pcm");
const FIXTURE_SHA256: &str = "961653bfc4a99686cca3f2d64f10c9fa5589cfb511098508ca916c7f20a826bd";

fn chunks(frames: &[InputFrame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .filter_map(|frame| {
            let InputFrame::Event(EventFrame {
                event: Event::AudioInput(audio),
            }) = frame
            else {
                return None;
            };
            audio
                .content
                .0
                .as_deref()
                .map(|text| STANDARD.decode(text).unwrap())
        })
        .collect()
}

pub(super) fn response_create_ready_audio() {
    let (mut input, ids) = new_input();
    assert!(send(&mut input, &ids, RESPONSE_CREATE).is_empty());
    assert_eq!(send(&mut input, &ids, SESSION_RATE_8000).len(), 2);
    assert_eq!(
        input.session.input_audio.sample_rate_hertz,
        Setting(Some(8000))
    );
    let frames = input
        .transform_realtime_request(RESPONSE_CREATE.as_bytes(), &ids)
        .unwrap();
    assert_eq!(frames.len(), 126);
    assert_eq!(wire(&frames)[0], audio_start(AUDIO, "16000"));
    assert_eq!(chunks(&frames).len(), 125);
    assert_eq!(input.open_audio, Some(Setting(Some(16000))));
    assert!(!input.session.client_audio_streamed);
    let repeated = input
        .transform_realtime_request(RESPONSE_CREATE.as_bytes(), &ids)
        .unwrap();
    assert_eq!(repeated.len(), 125, "an open content gets no second start");
    assert_eq!(chunks(&repeated), chunks(&frames));
    input.open_audio = Some(Setting(Some(24000)));
    let reused = input
        .transform_realtime_request(RESPONSE_CREATE.as_bytes(), &ids)
        .unwrap();
    assert_eq!(reused.len(), 125, "an open content is reused at any rate");
    assert_eq!(input.open_audio, Some(Setting(Some(24000))));
    let (mut streamed, streamed_ids) = new_input();
    send(&mut streamed, &streamed_ids, APPEND);
    assert_eq!(
        send(&mut streamed, &streamed_ids, SESSION_RATE_8000).len(),
        2
    );
    assert!(send(&mut streamed, &streamed_ids, RESPONSE_CREATE).is_empty());
}

pub(super) fn ready_trigger_fixture() {
    assert_eq!(FIXTURE.len(), 15990);
    assert_eq!(hex::encode(Sha256::digest(FIXTURE)), FIXTURE_SHA256);
    let (mut input, ids) = new_input();
    input.session.prompt_started = true;
    let frames = input
        .transform_realtime_request(RESPONSE_CREATE.as_bytes(), &ids)
        .unwrap();
    let pieces = chunks(&frames);
    assert_eq!(pieces.len(), 125);
    assert!(pieces.iter().take(124).all(|piece| piece.len() == 1024));
    assert_eq!(pieces.last().map(Vec::len), Some(1014));
    let pcm = pieces.concat();
    assert_eq!(pcm.len(), 127_990);
    assert!(pcm[..16_000].iter().all(|byte| *byte == 0));
    assert_eq!(&pcm[16_000..31_990], FIXTURE);
    assert_eq!(pcm[31_990..].len(), 96_000);
    assert!(pcm[31_990..].iter().all(|byte| *byte == 0));
}

pub(super) fn response_cancel() {
    let (captured, _guard) = capture();
    let (mut input, ids) = new_input();
    send(&mut input, &ids, APPEND);
    input.session.prompt_started = true;
    let names = input.session.names.audio_content.clone();
    assert!(send(&mut input, &ids, r#"{"type":"response.cancel"}"#).is_empty());
    assert!(captured.text().contains("Handling response.cancel"));
    assert!(input.session.prompt_started);
    assert!(input.session.client_audio_streamed);
    assert_eq!(input.open_audio, Some(Setting(Some(16000))));
    assert_eq!(input.session.names.audio_content, names);
}

pub(super) fn unknown_client_event() {
    let (captured, _guard) = capture();
    let (mut input, ids) = new_input();
    for broken in [
        "not json",
        r#"{"type":"input_audio_buffer.append","audio":"AAA"#,
        "",
    ] {
        assert!(send(&mut input, &ids, broken).is_empty());
    }
    let log = captured.text();
    assert_eq!(log.matches("Invalid JSON message").count(), 3);
    assert!(!log.contains("not json"));
    assert!(!log.contains("AAA"));
    for not_object in [
        "[]",
        "5",
        "null",
        r#""text""#,
        r#"[{"type":"response.create"}]"#,
    ] {
        let result = try_send(&mut input, &ids, not_object);
        assert!(matches!(result, Err(InputError::Malformed { .. })));
    }
    assert!(send(&mut input, &ids, "{}").is_empty());
    assert!(send(&mut input, &ids, r#"{"type":"response.delete"}"#).is_empty());
    assert!(send(&mut input, &ids, r#"{"type":5}"#).is_empty());
    assert!(send(&mut input, &ids, r#"{"type":null}"#).is_empty());
    let log = captured.text();
    assert!(log.contains("Unknown message type: None"));
    assert!(log.contains("Unknown message type: response.delete"));
    assert!(log.contains("Unknown message type: 5"));
    assert!(log.contains("Unknown message type: null"));
    let update = send(
        &mut input,
        &ids,
        r#"{"type":"session.update","session":{}}"#,
    );
    assert_eq!(update.len(), 2);
    assert!(input.session.prompt_started);
}
