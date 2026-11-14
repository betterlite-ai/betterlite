use rust_decimal::Decimal;

use super::support::{
    AUDIO_CONTENT, CONTENT, INSTRUCTION, PROMPT, apply, content_end, content_start, output_audio,
    prompt_start, session, session_start, text_input,
};
use crate::realtime_session::records::{
    ContentRole, ContentType, ExactDecimal, Infinite, MaxResponseTokens, Setting,
};

pub(super) fn session_update_options() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(&mut state, r#"{"type":"session.update"}"#, &[INSTRUCTION]);
    assert_eq!(frames.len(), 2);
    assert!(state.prompt_started);
    assert_eq!(
        session_start(&frames).inference_configuration.max_tokens,
        Setting(Some(MaxResponseTokens::Count(1024)))
    );
    assert_eq!(prompt_start(&frames).prompt_name, PROMPT);

    let frames = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":2048,"temperature":1.1,"voice":"amy","top_p":0.5}}"#,
        &[],
    );
    assert_eq!(
        session_start(&frames).inference_configuration.max_tokens,
        Setting(Some(MaxResponseTokens::Count(2048)))
    );
    assert_eq!(
        session_start(&frames).inference_configuration.temperature,
        Setting(Some(ExactDecimal(Decimal::new(11, 1))))
    );
    assert_eq!(
        session_start(&frames).inference_configuration.top_p,
        ExactDecimal(Decimal::new(9, 1))
    );
    assert_eq!(
        output_audio(&frames).voice_id,
        Setting(Some("amy".to_owned()))
    );

    let frames = apply(&mut state, r#"{"session":{}}"#, &[]);
    assert_eq!(
        session_start(&frames).inference_configuration.max_tokens,
        Setting(Some(MaxResponseTokens::Count(2048)))
    );
    assert_eq!(
        session_start(&frames).inference_configuration.temperature,
        Setting(Some(ExactDecimal(Decimal::new(11, 1))))
    );
    assert_eq!(
        output_audio(&frames).voice_id,
        Setting(Some("amy".to_owned()))
    );

    let frames = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":null,"temperature":null,"voice":""}}"#,
        &[],
    );
    assert_eq!(
        session_start(&frames).inference_configuration.max_tokens,
        Setting(None)
    );
    assert_eq!(
        session_start(&frames).inference_configuration.temperature,
        Setting(None)
    );
    assert_eq!(output_audio(&frames).voice_id, Setting(Some(String::new())));

    let frames = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":"inf"}}"#,
        &[],
    );
    assert_eq!(
        session_start(&frames).inference_configuration.max_tokens,
        Setting(Some(MaxResponseTokens::Infinite(Infinite::Inf)))
    );

    let frames = apply(
        &mut state,
        r#"{"session":{"unknown_field":1,"unknown_nested":{"a":1},"tools":[],"instructions":""}}"#,
        &[],
    );
    assert_eq!(
        session_start(&frames).inference_configuration.max_tokens,
        Setting(Some(MaxResponseTokens::Infinite(Infinite::Inf)))
    );
    assert!(prompt_start(&frames).tool_configuration.is_none());
    assert!(
        prompt_start(&frames)
            .tool_use_output_configuration
            .is_none()
    );
    assert_eq!(frames.len(), 2);

    let prompt_name = state.names.prompt.clone();
    let frames = apply(
        &mut state,
        r#"{"session":{"instructions":"be helpful"}}"#,
        &[INSTRUCTION],
    );
    assert_eq!(frames.len(), 5);
    assert_eq!(state.names.prompt, prompt_name);
    assert_eq!(state.names.content, CONTENT);
    assert_eq!(state.names.audio_content, AUDIO_CONTENT);
    assert!(state.prompt_started);
    assert_eq!(content_start(&frames).prompt_name, PROMPT);
    assert_eq!(content_start(&frames).content_name, INSTRUCTION);
    assert_eq!(content_start(&frames).content_type, ContentType::Text);
    assert_eq!(content_start(&frames).role, ContentRole::System);
    assert!(!content_start(&frames).interactive);
    assert_eq!(
        content_start(&frames).text_input_configuration.media_type,
        "text/plain"
    );
    assert_eq!(text_input(&frames), (PROMPT, INSTRUCTION, "be helpful"));
    assert_eq!(content_end(&frames), (PROMPT, INSTRUCTION));

    let frames = apply(&mut state, r#"{"session":{"instructions":""}}"#, &[]);
    assert_eq!(frames.len(), 2);
    let frames = apply(&mut state, r#"{"session":{"instructions":null}}"#, &[]);
    assert_eq!(frames.len(), 2);
    let frames = apply(&mut state, r#"{"session":{}}"#, &[]);
    assert_eq!(frames.len(), 2);
}

pub(super) fn system_instructions() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(
        &mut state,
        r#"{"session":{"instructions":"You are a helpful assistant."}}"#,
        &[INSTRUCTION],
    );
    assert_eq!(frames.len(), 5);
    assert_eq!(content_start(&frames).prompt_name, PROMPT);
    assert_eq!(content_start(&frames).content_name, INSTRUCTION);
    assert_eq!(content_start(&frames).content_type, ContentType::Text);
    assert_eq!(content_start(&frames).role, ContentRole::System);
    assert!(!content_start(&frames).interactive);
    assert_eq!(
        content_start(&frames).text_input_configuration.media_type,
        "text/plain"
    );
    assert_eq!(
        text_input(&frames),
        (PROMPT, INSTRUCTION, "You are a helpful assistant.")
    );
    assert_eq!(content_end(&frames), (PROMPT, INSTRUCTION));

    let mut empty = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(&mut empty, r#"{"session":{"instructions":""}}"#, &[]);
    assert_eq!(frames.len(), 2);
    let mut absent = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(&mut absent, r#"{"session":{}}"#, &[]);
    assert_eq!(frames.len(), 2);
}
