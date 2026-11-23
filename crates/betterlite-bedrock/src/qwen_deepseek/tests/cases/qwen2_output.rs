#![cfg(test)]

use crate::qwen_deepseek::QwenProjection;
use crate::qwen_deepseek::error::QwenReplyError;
use crate::qwen_deepseek::tests::support::{fixture, qwen, two_choices};

const BOTH: [QwenProjection; 2] = [QwenProjection::Qwen2, QwenProjection::Qwen3];

pub(in crate::qwen_deepseek::tests) fn qwen2_output() {
    generation_preferred();
    text_fallback();
    qwen3_generation_only();
    wrappers_removed_once();
    first_choice_only();
    empty_choices_untouched();
}

fn generation_preferred() {
    let case = fixture();
    let run = qwen(
        &case,
        QwenProjection::Qwen2,
        r#"{"generation": "from generation", "text": "from text"}"#,
    );
    assert_eq!(
        run.reply().choices[0].message.content.as_deref(),
        Some("from generation"),
        "Qwen2 reads generation before text"
    );
    assert_eq!(
        run.reply().choices[0].finish_reason.as_deref(),
        Some("stop")
    );
}

fn text_fallback() {
    let case = fixture();
    for (body, expected) in [
        (r#"{"text": "only text"}"#, "only text"),
        (
            r#"{"generation": "", "text": "empty generation"}"#,
            "empty generation",
        ),
        (
            r#"{"generation": null, "text": "null generation"}"#,
            "null generation",
        ),
        (
            r#"{"generation": 0, "text": "zero generation"}"#,
            "zero generation",
        ),
        (
            r#"{"generation": 0.0, "text": "zero float generation"}"#,
            "zero float generation",
        ),
        (
            r#"{"generation": false, "text": "false generation"}"#,
            "false generation",
        ),
        (
            r#"{"generation": [], "text": "empty list generation"}"#,
            "empty list generation",
        ),
        (
            r#"{"generation": {}, "text": "empty object generation"}"#,
            "empty object generation",
        ),
        (r#"{"generation": "0", "text": "truthy"}"#, "0"),
    ] {
        let run = qwen(&case, QwenProjection::Qwen2, body);
        assert_eq!(
            run.reply().choices[0].message.content.as_deref(),
            Some(expected),
            "{body}"
        );
    }
    let missing = qwen(&case, QwenProjection::Qwen2, "{}");
    assert_eq!(
        missing.reply().choices[0].message.content.as_deref(),
        Some(""),
        "neither key is empty content"
    );
}

fn qwen3_generation_only() {
    let case = fixture();
    for projection in BOTH {
        let run = qwen(
            &case,
            projection,
            r#"{"generation": "gen3", "text": "ignored text"}"#,
        );
        assert_eq!(
            run.reply().choices[0].message.content.as_deref(),
            Some("gen3"),
            "{projection:?}"
        );
    }
    let missing = qwen(&case, QwenProjection::Qwen3, r#"{"text": "ignored text"}"#);
    assert_eq!(
        missing.reply().choices[0].message.content.as_deref(),
        Some(""),
        "Qwen3 does not fall back to text"
    );
    for body in [
        r#"{"generation": null}"#,
        r#"{"generation": 0, "text": "ignored"}"#,
        r#"{"generation": false, "text": "ignored"}"#,
    ] {
        let run = qwen(&case, QwenProjection::Qwen3, body);
        assert!(
            matches!(run.error(), QwenReplyError::NotText { field } if *field == "generation"),
            "Qwen3 has no truthiness fallback for {body}"
        );
    }
}

fn wrappers_removed_once() {
    let case = fixture();
    for (body, expected) in [
        (
            r#"{"generation": "<|im_start|>assistant\nhello<|im_end|>"}"#,
            "hello",
        ),
        (
            r#"{"generation": "<|im_start|>assistant\n<|im_start|>assistant\nhello<|im_end|><|im_end|>"}"#,
            "<|im_start|>assistant\nhello<|im_end|>",
        ),
        (
            r#"{"generation": "<|im_start|>assistant\n  hello  <|im_end|>"}"#,
            "  hello  ",
        ),
        (r#"{"generation": "hello"}"#, "hello"),
        (r#"{"generation": "<|im_start|>assistant\nhello"}"#, "hello"),
        (r#"{"generation": "hello<|im_end|>"}"#, "hello"),
        (
            r#"{"generation": "<|im_end|>hello<|im_start|>assistant\n"}"#,
            "<|im_end|>hello<|im_start|>assistant\n",
        ),
    ] {
        for projection in BOTH {
            let run = qwen(&case, projection, body);
            assert_eq!(
                run.reply().choices[0].message.content.as_deref(),
                Some(expected),
                "{projection:?} {body}"
            );
        }
    }
    let fallback = qwen(
        &case,
        QwenProjection::Qwen2,
        r#"{"text": "<|im_start|>assistant\nfallback<|im_end|>"}"#,
    );
    assert_eq!(
        fallback.reply().choices[0].message.content.as_deref(),
        Some("fallback"),
        "the text fallback is unwrapped too"
    );
}

fn first_choice_only() {
    let mut case = fixture();
    case.response = two_choices();
    for projection in BOTH {
        let run = qwen(&case, projection, r#"{"generation": "new"}"#);
        let reply = run.reply();
        assert_eq!(reply.id, "chatcmpl-supplied", "the id is kept");
        assert_eq!(reply.object, "chat.completion", "the object is kept");
        assert_eq!(reply.created, 1, "the creation time is kept");
        assert_eq!(reply.model, "previous-model", "the model is kept");
        assert_eq!(reply.choices.len(), 2);
        assert_eq!(reply.choices[0].index, 0);
        assert_eq!(
            reply.choices[0].message.content.as_deref(),
            Some("new"),
            "{projection:?}"
        );
        assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(reply.choices[1].index, 1);
        assert_eq!(reply.choices[1].message.content.as_deref(), Some("second"));
        assert_eq!(reply.choices[1].finish_reason.as_deref(), Some("length"));
    }
}

fn empty_choices_untouched() {
    let mut case = fixture();
    case.response.choices.clear();
    let run = qwen(&case, QwenProjection::Qwen3, r#"{"generation": "ignored"}"#);
    assert!(run.reply().choices.is_empty(), "empty choices stay empty");
    assert!(!run.logged(), "an empty-choices reply is not a failure");
}
