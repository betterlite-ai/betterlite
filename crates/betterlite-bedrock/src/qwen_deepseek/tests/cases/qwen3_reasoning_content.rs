#![cfg(test)]

use crate::qwen_deepseek::QwenProjection;
use crate::qwen_deepseek::tests::support::{fixture, object, qwen};

pub(in crate::qwen_deepseek::tests) fn qwen3_reasoning_content() {
    tags_stay_visible();
    existing_reasoning_fields_stay();
}

fn tags_stay_visible() {
    let case = fixture();
    for projection in [QwenProjection::Qwen2, QwenProjection::Qwen3] {
        let run = qwen(
            &case,
            projection,
            r#"{"generation": "<|im_start|>assistant\n<think>\nstep one\n</think>\nanswer<|im_end|>"}"#,
        );
        let reply = run.reply();
        assert_eq!(
            reply.choices[0].message.content.as_deref(),
            Some("<think>\nstep one\n</think>\nanswer"),
            "{projection:?} keeps the tags in the visible content"
        );
        assert_eq!(
            reply.choices[0].message.provider_specific_fields, None,
            "the reply gains no reasoning field"
        );
    }
}

fn existing_reasoning_fields_stay() {
    let mut case = fixture();
    case.response.choices[0].message.provider_specific_fields =
        Some(object(r#"{"reasoning_content": "existing", "other": 1}"#));
    case.response.choices[0].provider_specific_fields = Some(object(r#"{"choice_field": true}"#));
    let run = qwen(
        &case,
        QwenProjection::Qwen3,
        r#"{"generation": "<think>visible</think>"}"#,
    );
    let reply = run.reply();
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some("<think>visible</think>")
    );
    assert_eq!(
        reply.choices[0].message.provider_specific_fields,
        Some(object(r#"{"reasoning_content": "existing", "other": 1}"#)),
        "the message reasoning fields stay as sent"
    );
    assert_eq!(
        reply.choices[0].provider_specific_fields,
        Some(object(r#"{"choice_field": true}"#)),
        "the choice fields stay as sent"
    );
}
