use super::support::{
    AUDIO, PROMPT, SESSION_END, audio_input, audio_start, content_end, id, new_input, prompt_end,
    send, wire,
};
use crate::realtime_session::records::Setting;

const APPEND: &str = r#"{"type":"input_audio_buffer.append","audio":"QUJD"}"#;
const COMMIT: &str = r#"{"type":"input_audio_buffer.commit"}"#;
const UPDATE: &str = r#"{"type":"session.update","session":{}}"#;
const RATE_8000: &str = r#"{"type":"session.update","session":{"input_sample_rate_hertz":8000}}"#;

pub(super) fn audio_append_before_after_session() {
    let (mut input, ids) = new_input();
    assert!(!input.session.prompt_started);
    assert_eq!(
        send(&mut input, &ids, APPEND),
        [audio_start(AUDIO, "16000"), audio_input(AUDIO, r#""QUJD""#)]
    );
    assert!(input.session.client_audio_streamed);
    assert_eq!(input.open_audio, Some(Setting(Some(16000))));
    let not_base64 = r#"{"type":"input_audio_buffer.append","audio":"not base64!"}"#;
    assert_eq!(
        send(&mut input, &ids, not_base64),
        [audio_input(AUDIO, r#""not base64!""#)]
    );
    let missing = r#"{"type":"input_audio_buffer.append"}"#;
    assert_eq!(
        send(&mut input, &ids, missing),
        [audio_input(AUDIO, r#""""#)]
    );
    let null = r#"{"type":"input_audio_buffer.append","audio":null}"#;
    assert_eq!(send(&mut input, &ids, null), [audio_input(AUDIO, "null")]);
    assert_eq!(send(&mut input, &ids, UPDATE).len(), 2);
    assert!(input.session.prompt_started);
    assert_eq!(
        send(&mut input, &ids, APPEND),
        [audio_input(AUDIO, r#""QUJD""#)]
    );
    let (mut late, late_ids) = new_input();
    send(&mut late, &late_ids, UPDATE);
    assert!(!late.session.client_audio_streamed);
    assert_eq!(
        send(&mut late, &late_ids, APPEND),
        [audio_start(AUDIO, "16000"), audio_input(AUDIO, r#""QUJD""#)]
    );
    assert!(late.session.client_audio_streamed);
}

pub(super) fn audio_commit() {
    let (mut input, ids) = new_input();
    assert!(send(&mut input, &ids, COMMIT).is_empty());
    assert!(!input.session.client_audio_streamed);
    send(&mut input, &ids, APPEND);
    assert_eq!(send(&mut input, &ids, COMMIT), [content_end(AUDIO)]);
    assert_eq!(input.open_audio, None);
    assert!(input.session.client_audio_streamed);
    assert_eq!(input.session.names.audio_content, AUDIO);
    assert!(send(&mut input, &ids, COMMIT).is_empty());
    assert_eq!(
        send(&mut input, &ids, APPEND),
        [audio_start(AUDIO, "16000"), audio_input(AUDIO, r#""QUJD""#)]
    );
}

pub(super) fn audio_sample_rate_transition() {
    let (mut input, ids) = new_input();
    send(&mut input, &ids, APPEND);
    send(&mut input, &ids, RATE_8000);
    let fresh = id(4);
    assert_eq!(
        send(&mut input, &ids, APPEND),
        [
            content_end(AUDIO),
            audio_start(&fresh, "8000"),
            audio_input(&fresh, r#""QUJD""#)
        ]
    );
    assert_eq!(input.session.names.audio_content, fresh);
    assert_eq!(input.open_audio, Some(Setting(Some(8000))));
    send(&mut input, &ids, COMMIT);
    assert_eq!(
        send(&mut input, &ids, APPEND),
        [
            audio_start(&fresh, "8000"),
            audio_input(&fresh, r#""QUJD""#)
        ]
    );
    let to_null = r#"{"type":"session.update","session":{"input_sample_rate_hertz":null}}"#;
    send(&mut input, &ids, to_null);
    let next = id(5);
    assert_eq!(
        send(&mut input, &ids, APPEND),
        [
            content_end(&fresh),
            audio_start(&next, "null"),
            audio_input(&next, r#""QUJD""#)
        ]
    );
}

pub(super) fn close_session_order() {
    let (mut input, ids) = new_input();
    assert!(input.session_close_messages().is_empty());
    send(&mut input, &ids, APPEND);
    assert!(input.session_close_messages().is_empty());
    assert_eq!(input.open_audio, Some(Setting(Some(16000))));
    send(&mut input, &ids, UPDATE);
    assert_eq!(
        wire(&input.session_close_messages()),
        [content_end(AUDIO), prompt_end(), SESSION_END.to_owned()]
    );
    assert!(!input.session.prompt_started);
    assert_eq!(input.open_audio, None);
    assert!(input.session.client_audio_streamed);
    assert_eq!(input.session.names.audio_content, AUDIO);
    assert!(input.session_close_messages().is_empty());
    send(&mut input, &ids, UPDATE);
    assert_eq!(
        wire(&input.session_close_messages()),
        [prompt_end(), SESSION_END.to_owned()]
    );
    assert_eq!(input.session.names.prompt, PROMPT);
}
