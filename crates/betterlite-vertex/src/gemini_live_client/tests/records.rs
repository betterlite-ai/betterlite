#![cfg(test)]

use crate::gemini_live_client::records::{ServerContent, ServerMessage};

#[test]
fn a_server_message_decodes_the_fields_betterlite_reads() {
    let message: ServerMessage = serde_json::from_str(
        r#"{
            "usageMetadata": {"promptTokenCount": 3},
            "serverContent": {
                "turnComplete": true,
                "modelTurn": {
                    "role": "model",
                    "parts": [
                        {"text": "hi"},
                        {"inlineData": {"mimeType": "audio/pcm;rate=24000", "data": "QUJD"}}
                    ]
                }
            },
            "setupComplete": {}
        }"#,
    )
    .expect("the frame decodes");
    assert!(message.usage_metadata.is_some());
    assert!(message.setup_complete.is_some());
    let content: ServerContent = message.server_content.expect("the content is present");
    assert_eq!(content.turn_complete, Some(true));
    let turn = content.model_turn.expect("the model turn is present");
    assert_eq!(turn.role.as_deref(), Some("model"));
    let parts = turn.parts.expect("the parts are present");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].text.as_deref(), Some("hi"));
    let inline = parts[1].inline_data.as_ref().expect("the audio is present");
    assert_eq!(inline.mime_type.as_deref(), Some("audio/pcm;rate=24000"));
    assert_eq!(inline.data.as_deref(), Some("QUJD"));
}

#[test]
fn a_server_message_leaves_out_the_fields_it_does_not_carry() {
    let message: ServerMessage =
        serde_json::from_str(r#"{"serverContent":{"inputTranscription":{"text":"hello"}}}"#)
            .expect("the frame decodes");
    let content = message.server_content.expect("the content is present");
    assert_eq!(
        content
            .input_transcription
            .and_then(|transcription| transcription.text),
        Some("hello".to_owned())
    );
    assert_eq!(content.turn_complete, None);
}

#[test]
fn a_server_message_ignores_provider_keys_it_does_not_read() {
    let message: ServerMessage =
        serde_json::from_str(r#"{"serverContent":{"interrupted":true,"unknown":1},"extra":{}}"#)
            .expect("the frame decodes");
    let content = message.server_content.expect("the content is present");
    assert_eq!(content.interrupted, Some(true));
}
