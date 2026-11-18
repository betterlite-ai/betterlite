use betterlite_prompts::bedrock_processor::error::ProcessorError;

use crate::converse_options::error::ConverseOptionsError;
use crate::converse_request::error::ConverseRequestError;

use super::support::{
    ARN, CLAUDE_3_5, CLAUDE_4_5, Call, LLAMA, Settings, expected, field, processor_error, remote,
    run,
};

const SCRIPT: &str = r#"[
    {"role": "system", "content": "be brief"},
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": "partial"}
]"#;

const ORDER_OPTIONS: &str = r#"{
    "guardrailConfig": {"guardrailIdentifier": "gr-1"},
    "requestMetadata": {"client_key": "caller", "user_api_key_alias": "spoofed"}
}"#;

const ORDER_LITELLM: &str = r#"{
    "user_continue_message": {"role": "user", "content": "continue"},
    "metadata": {"user_api_key_alias": "alias-1"}
}"#;

const ORDER_BODY: &str = r#"{
    "messages": [
        {"role": "user", "content": [{"text": "hi"}]},
        {"role": "assistant", "content": [{"text": "partial"}]},
        {"role": "user", "content": [{"text": "continue"}]}
    ],
    "inferenceConfig": {},
    "system": [{"text": "be brief"}],
    "guardrailConfig": {"guardrailIdentifier": "gr-1"},
    "requestMetadata": {"user_api_key_alias": "alias-1", "client_key": "caller"}
}"#;

const ORDER_LITELLM_AFTER: &str = r#"{"metadata": {"user_api_key_alias": "alias-1"}}"#;

const GUARDED_MESSAGES: &str =
    r#"[{"role": "user", "content": [{"guardContent": {"text": {"text": "hi"}}}]}]"#;

// System extraction runs first, then the continue message, then metadata ownership replaces the caller's.
pub(super) fn system_message_order() {
    system_extracted_and_continue_appended();
    metadata_ownership_empties_caller_copy();
    guardrail_rewrites_trailing_user_turn();
}

fn system_extracted_and_continue_appended() {
    let call = Call {
        messages: SCRIPT,
        litellm_params: Some(ORDER_LITELLM),
        policy: Some(&["user_api_key_alias"]),
        ..Call::new(CLAUDE_3_5, ORDER_OPTIONS)
    };
    let (outcome, _warnings) = run(&call, &remote(&[]));
    assert_eq!(outcome.body(), &expected(ORDER_BODY));
    assert_eq!(outcome.litellm_params, Some(expected(ORDER_LITELLM_AFTER)));
}

fn metadata_ownership_empties_caller_copy() {
    let call = Call {
        policy: Some(&["user_api_key_alias"]),
        ..Call::new(
            CLAUDE_3_5,
            r#"{"requestMetadata": {"user_api_key_alias": "spoofed"}}"#,
        )
    };
    let (emptied, _warnings) = run(&call, &remote(&[]));
    assert!(field(emptied.body(), "requestMetadata").is_none());
}

fn guardrail_rewrites_trailing_user_turn() {
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
        Some(&expected(GUARDED_MESSAGES))
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
const EFFORT_OPTIONS: &str = r#"{"output_config": {"effort": "high"}}"#;
const EFFORT_ADDITIONAL: &str =
    r#"{"output_config": {"effort": "high"}, "anthropic_beta": ["effort-2025-11-24"]}"#;
const EFFORT_CEILING_ANSWERS: &str =
    r#"{"anthropic.claude-4-5-test": {"bedrock_output_config_effort_ceiling": "high"}}"#;
const MAX_EFFORT_OPTIONS: &str = r#"{"output_config": {"effort": "max"}}"#;
const NORMALIZED_MAX_EFFORT_OPTIONS: &str =
    r#"{"_output_config_normalized": true, "output_config": {"effort": "max"}}"#;

// The application inference profile ARN forwards remaining output_config verbatim and skips both gates.
pub(super) fn model_arn_request() {
    arn_forwards_output_config();
    global_drop_warns_on_unadvertised_effort();
    per_call_drop_forwards_output_config();
    foreign_model_drops_output_config_silently();
    discarded_format_still_warns_before_arn();
    effort_ceiling_clamps_before_validation();
    normalized_marker_skips_the_clamp();
}

fn arn_forwards_output_config() {
    let (arn, warnings) = run(&Call::new(ARN, EFFORT_OPTIONS), &remote(&[]));
    assert_eq!(
        field(arn.body(), "additionalModelRequestFields"),
        Some(&expected(EFFORT_OPTIONS))
    );
    assert!(field(arn.body(), "outputConfig").is_none());
    assert!(warnings.is_empty(), "{warnings}");
}

fn global_drop_warns_on_unadvertised_effort() {
    let call = Call {
        settings: Settings {
            global_drop: true,
            modify_params: false,
            neutralize: false,
        },
        ..Call::new(CLAUDE_4_5, EFFORT_OPTIONS)
    };
    let (dropped, warnings) = run(&call, &remote(&[]));
    assert!(field(dropped.body(), "additionalModelRequestFields").is_none());
    assert!(warnings.contains(DROP_OUTPUT_CONFIG_WARNING), "{warnings}");
    assert!(warnings.contains(CLAUDE_4_5), "{warnings}");
}

fn per_call_drop_forwards_output_config() {
    let call = Call {
        litellm_params: Some(r#"{"drop_params": true}"#),
        ..Call::new(CLAUDE_4_5, EFFORT_OPTIONS)
    };
    let (per_call, warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(per_call.body(), "additionalModelRequestFields"),
        Some(&expected(EFFORT_ADDITIONAL))
    );
    assert!(!warnings.contains(DROP_OUTPUT_CONFIG_WARNING), "{warnings}");
}

fn foreign_model_drops_output_config_silently() {
    let (foreign, warnings) = run(&Call::new(LLAMA, EFFORT_OPTIONS), &remote(&[]));
    assert!(field(foreign.body(), "additionalModelRequestFields").is_none());
    assert!(warnings.is_empty(), "{warnings}");
}

fn discarded_format_still_warns_before_arn() {
    let (removed, warnings) = run(
        &Call::new(
            ARN,
            r#"{"output_config": {"format": {"type": "text"}, "effort": "high"}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(removed.body(), "additionalModelRequestFields"),
        Some(&expected(EFFORT_OPTIONS))
    );
    assert!(warnings.contains(FORMAT_WARNING), "{warnings}");
}

fn effort_ceiling_clamps_before_validation() {
    let call = Call {
        answers: EFFORT_CEILING_ANSWERS,
        ..Call::new(CLAUDE_4_5, MAX_EFFORT_OPTIONS)
    };
    let (clamped, warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(clamped.body(), "additionalModelRequestFields"),
        Some(&expected(EFFORT_ADDITIONAL))
    );
    assert!(!warnings.contains(DROP_OUTPUT_CONFIG_WARNING), "{warnings}");
}

fn normalized_marker_skips_the_clamp() {
    // A caller that already normalized keeps its value, so the effort gate rejects max on this model.
    let call = Call {
        answers: EFFORT_CEILING_ANSWERS,
        ..Call::new(CLAUDE_4_5, NORMALIZED_MAX_EFFORT_OPTIONS)
    };
    let (raw, _warnings) = run(&call, &remote(&[]));
    assert!(matches!(
        raw.error(),
        ConverseRequestError::Options(ConverseOptionsError::UnsupportedEffort { .. })
    ));
}

const NATIVE_CONFIG: &str = r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {
    "schema": "{}",
    "name": "out"
}}}}"#;

// The native outputConfig comes from a json_schema format candidate unless camel outputConfig is set.
pub(super) fn output_format_precedence() {
    json_schema_builds_native_config();
    explicit_output_config_wins();
    unsupported_schema_warns();
    scalar_format_is_discarded();
}

fn json_schema_builds_native_config() {
    let (native, warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{"output_config": {"format": {"type": "json_schema", "name": "out"}, "effort": "high"}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(native.body(), "outputConfig"),
        Some(&expected(NATIVE_CONFIG))
    );
    assert_eq!(
        field(native.body(), "additionalModelRequestFields"),
        Some(&expected(EFFORT_ADDITIONAL))
    );
    assert!(!warnings.contains(FORMAT_WARNING), "{warnings}");
}

fn explicit_output_config_wins() {
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
}

fn unsupported_schema_warns() {
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
}

fn scalar_format_is_discarded() {
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
