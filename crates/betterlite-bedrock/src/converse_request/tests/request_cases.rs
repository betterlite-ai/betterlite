use betterlite_prompts::bedrock_processor::error::ProcessorError;

use super::support::{
    ARN, CLAUDE_3_5, CLAUDE_4_5, Call, LLAMA, Settings, expected, field, processor_error, remote,
    run,
};

const SCRIPT: &str = r#"[
    {"role": "system", "content": "be brief"},
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": "partial"}
]"#;

// System extraction runs first, then the continue message, then metadata ownership replaces the caller's.
pub(super) fn system_message_order() {
    let optional = r#"{
        "guardrailConfig": {"guardrailIdentifier": "gr-1"},
        "requestMetadata": {"client_key": "caller", "user_api_key_alias": "spoofed"}
    }"#;
    let litellm = r#"{
        "user_continue_message": {"role": "user", "content": "continue"},
        "metadata": {"user_api_key_alias": "alias-1"}
    }"#;
    let call = Call {
        messages: SCRIPT,
        litellm_params: Some(litellm),
        policy: Some(&["user_api_key_alias"]),
        ..Call::new(CLAUDE_3_5, optional)
    };
    let (outcome, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        outcome.body(),
        &expected(
            r#"{
                "messages": [
                    {"role": "user", "content": [{"text": "hi"}]},
                    {"role": "assistant", "content": [{"text": "partial"}]},
                    {"role": "user", "content": [{"text": "continue"}]}
                ],
                "inferenceConfig": {},
                "system": [{"text": "be brief"}],
                "guardrailConfig": {"guardrailIdentifier": "gr-1"},
                "requestMetadata": {"user_api_key_alias": "alias-1", "client_key": "caller"}
            }"#,
        )
    );
    assert_eq!(
        outcome.litellm_params,
        Some(expected(
            r#"{"metadata": {"user_api_key_alias": "alias-1"}}"#
        ))
    );

    // Ownership removes the caller's copy even when nothing survives the reserved-prefix filter.
    let call = Call {
        policy: Some(&["user_api_key_alias"]),
        ..Call::new(
            CLAUDE_3_5,
            r#"{"requestMetadata": {"user_api_key_alias": "spoofed"}}"#,
        )
    };
    let (emptied, _warnings) = run(&call, &remote(&[]));
    assert!(field(emptied.body(), "requestMetadata").is_none());

    // The guardrail rewrites the trailing user turn while guardrailConfig is still in the options.
    let call = Call {
        messages: r#"[{"role": "system", "content": "top"}, {"role": "user", "content": "hi"}]"#,
        ..Call::new(
            CLAUDE_3_5,
            r#"{"guardrailConfig": {"guardrailIdentifier": "gr-1"}}"#,
        )
    };
    let (guarded, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(guarded.body(), "messages"),
        Some(&expected(
            r#"[{"role": "user", "content": [{"guardContent": {"text": {"text": "hi"}}}]}]"#,
        ))
    );
    assert_eq!(
        field(guarded.body(), "system"),
        Some(&expected(r#"[{"text": "top"}]"#))
    );
}

const MEDIA: &str = r#"[{"role": "user", "content": [
    {"type": "image_url", "image_url": "data:image/png;base64,AAAA"},
    {"type": "file", "file": {
        "file_data": "data:application/pdf;base64,JVBERi0=",
        "format": "application/pdf"
    }}
]}]"#;

// Supplied bytes convert without a socket; a document url source is refused before transport.
pub(super) fn inline_document_image() {
    let call = Call {
        messages: MEDIA,
        ..Call::new(CLAUDE_3_5, "{}")
    };
    let (outcome, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(outcome.body(), "messages"),
        Some(&expected(
            r#"[{"role": "user", "content": [
                {"image": {"format": "png", "source": {"bytes": "AAAA"}}},
                {"document": {
                    "format": "pdf",
                    "source": {"bytes": "JVBERi0="},
                    "name": "DocumentPDFmessages_fdaa7659473d3816_pdf"
                }},
                {"text": "."}
            ]}]"#,
        ))
    );

    let call = Call {
        messages: r#"[{"role": "user", "content": [{"type": "document", "source": {
            "type": "url",
            "url": "https://example.com/a.pdf",
            "media_type": "application/pdf"
        }}]}]"#,
        ..Call::new(CLAUDE_3_5, "{}")
    };
    let (refused, _warnings) = run(&call, &remote(&[]));
    assert!(matches!(
        processor_error(refused.error()),
        Some(ProcessorError::DocumentSourceNotBase64 { block: 0 })
    ));
}

const FORMAT_WARNING: &str = "does not advertise `supports_native_structured_output` in model_prices_and_context_window.json. The schema will not be enforced; pass `response_format` to use the synthetic tool-call fallback.";
const DROP_OUTPUT_CONFIG_WARNING: &str = "Dropping unsupported `output_config` for model=";

// The application inference profile ARN forwards remaining output_config verbatim and skips both gates.
pub(super) fn model_arn_request() {
    let (arn, warnings) = run(
        &Call::new(ARN, r#"{"output_config": {"effort": "high"}}"#),
        &remote(&[]),
    );
    assert_eq!(
        field(arn.body(), "additionalModelRequestFields"),
        Some(&expected(r#"{"output_config": {"effort": "high"}}"#))
    );
    assert!(field(arn.body(), "outputConfig").is_none());
    assert!(warnings.is_empty(), "{warnings}");

    // Global drop with a model that advertises no effort level warns and omits the field.
    let call = Call {
        settings: Settings {
            global_drop: true,
            modify_params: false,
            neutralize: false,
        },
        ..Call::new(CLAUDE_4_5, r#"{"output_config": {"effort": "high"}}"#)
    };
    let (dropped, warnings) = run(&call, &remote(&[]));
    assert!(field(dropped.body(), "additionalModelRequestFields").is_none());
    assert!(warnings.contains(DROP_OUTPUT_CONFIG_WARNING), "{warnings}");
    assert!(warnings.contains(CLAUDE_4_5), "{warnings}");

    // The per-call flag alone does not select that branch: the config is forwarded.
    let call = Call {
        litellm_params: Some(r#"{"drop_params": true}"#),
        ..Call::new(CLAUDE_4_5, r#"{"output_config": {"effort": "high"}}"#)
    };
    let (per_call, warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(per_call.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{"output_config": {"effort": "high"}, "anthropic_beta": ["effort-2025-11-24"]}"#,
        ))
    );
    assert!(!warnings.contains(DROP_OUTPUT_CONFIG_WARNING), "{warnings}");

    // A model outside the Anthropic family drops the remaining config without a warning.
    let (foreign, warnings) = run(
        &Call::new(LLAMA, r#"{"output_config": {"effort": "high"}}"#),
        &remote(&[]),
    );
    assert!(field(foreign.body(), "additionalModelRequestFields").is_none());
    assert!(warnings.is_empty(), "{warnings}");

    // Format removal happens before the ARN forwarding, so the discarded format still warns.
    let (removed, warnings) = run(
        &Call::new(
            ARN,
            r#"{"output_config": {"format": {"type": "text"}, "effort": "high"}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(removed.body(), "additionalModelRequestFields"),
        Some(&expected(r#"{"output_config": {"effort": "high"}}"#))
    );
    assert!(warnings.contains(FORMAT_WARNING), "{warnings}");
}

// The native outputConfig comes from a json_schema format candidate unless camel outputConfig is set.
pub(super) fn output_format_precedence() {
    let (native, warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{"output_config": {"format": {"type": "json_schema", "name": "out"}, "effort": "high"}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(native.body(), "outputConfig"),
        Some(&expected(
            r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {
                "schema": "{}",
                "name": "out"
            }}}}"#,
        ))
    );
    assert_eq!(
        field(native.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{"output_config": {"effort": "high"}, "anthropic_beta": ["effort-2025-11-24"]}"#,
        ))
    );
    assert!(!warnings.contains(FORMAT_WARNING), "{warnings}");

    // An explicit camel outputConfig wins over the format candidate, which drops silently.
    let (camel, warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{
                "outputConfig": {"textFormat": {"type": "text"}},
                "output_config": {"format": {"type": "text"}}
            }"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(camel.body(), "outputConfig"),
        Some(&expected(r#"{"textFormat": {"type": "text"}}"#))
    );
    assert!(field(camel.body(), "additionalModelRequestFields").is_none());
    assert!(!warnings.contains(FORMAT_WARNING), "{warnings}");

    // A json_schema format on a model without the native capability warns and builds nothing.
    for model in [CLAUDE_4_5, LLAMA] {
        let (unsupported, warnings) = run(
            &Call::new(
                model,
                r#"{"output_config": {"format": {"type": "json_schema"}}}"#,
            ),
            &remote(&[]),
        );
        assert!(field(unsupported.body(), "outputConfig").is_none());
        assert!(warnings.contains(FORMAT_WARNING), "{model}: {warnings}");
    }

    // A format that is not an object is discarded without the warning.
    let (scalar, warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{"output_config": {"format": "json_schema"}}"#,
        ),
        &remote(&[]),
    );
    assert!(field(scalar.body(), "outputConfig").is_none());
    assert!(field(scalar.body(), "additionalModelRequestFields").is_none());
    assert!(!warnings.contains(FORMAT_WARNING), "{warnings}");
}
