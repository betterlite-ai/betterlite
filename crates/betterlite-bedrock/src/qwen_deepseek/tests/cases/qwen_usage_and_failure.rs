#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::qwen_deepseek::QwenProjection;
use crate::qwen_deepseek::error::QwenReplyError;
use crate::qwen_deepseek::reply::Usage;
use crate::qwen_deepseek::tests::support::{fixture, qwen, two_choices};

const BOTH: [QwenProjection; 2] = [QwenProjection::Qwen2, QwenProjection::Qwen3];

pub(in crate::qwen_deepseek::tests) fn qwen_usage_and_failure() {
    present_usage_replaces();
    counts_default_independently();
    missing_usage_is_preserved();
    usage_errors();
    json_and_shape_errors();
    successful_replies_log_nothing();
}

fn present_usage_replaces() {
    let case = fixture();
    for projection in BOTH {
        let run = qwen(
            &case,
            projection,
            r#"{"generation": "x", "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 99}}"#,
        );
        assert_eq!(
            run.reply().usage,
            Some(Usage {
                prompt_tokens: 2,
                completion_tokens: 3,
                total_tokens: 99,
            }),
            "{projection:?} keeps the reported total"
        );
    }
}

fn counts_default_independently() {
    let case = fixture();
    for (body, expected) in [
        (
            r#"{"generation": "x", "usage": {}}"#,
            Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
        ),
        (
            r#"{"generation": "x", "usage": {"prompt_tokens": 5}}"#,
            Usage {
                prompt_tokens: 5,
                completion_tokens: 0,
                total_tokens: 0,
            },
        ),
        (
            r#"{"generation": "x", "usage": {"completion_tokens": 4}}"#,
            Usage {
                prompt_tokens: 0,
                completion_tokens: 4,
                total_tokens: 0,
            },
        ),
        (
            r#"{"generation": "x", "usage": {"total_tokens": 7}}"#,
            Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 7,
            },
        ),
        (
            r#"{"generation": "x", "usage": {"prompt_tokens": 2, "completion_tokens": 3}}"#,
            Usage {
                prompt_tokens: 2,
                completion_tokens: 3,
                total_tokens: 0,
            },
        ),
        (
            r#"{"generation": "x", "usage": {"prompt_tokens": -1}}"#,
            Usage {
                prompt_tokens: -1,
                completion_tokens: 0,
                total_tokens: 0,
            },
        ),
    ] {
        for projection in BOTH {
            let run = qwen(&case, projection, body);
            assert_eq!(
                run.reply().usage.as_ref(),
                Some(&expected),
                "{projection:?} {body}"
            );
        }
    }
}

fn missing_usage_is_preserved() {
    let mut case = fixture();
    case.response = two_choices();
    for projection in BOTH {
        let run = qwen(&case, projection, r#"{"generation": "x"}"#);
        assert_eq!(
            run.reply().usage,
            Some(Usage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
            }),
            "{projection:?} keeps the seeded usage"
        );
        let other = qwen(&case, projection, r#"{"generation": "x", "other": 1}"#);
        assert_eq!(other.reply().usage, run.reply().usage);
    }
    let plain = fixture();
    let run = qwen(&plain, QwenProjection::Qwen3, r#"{"generation": "x"}"#);
    assert_eq!(run.reply().usage, None, "an absent usage stays absent");
}

fn usage_errors() {
    let case = fixture();
    for body in [
        r#"{"generation": "x", "usage": null}"#,
        r#"{"generation": "x", "usage": []}"#,
        r#"{"generation": "x", "usage": "x"}"#,
    ] {
        for projection in BOTH {
            let run = qwen(&case, projection, body);
            assert!(
                matches!(run.error(), QwenReplyError::UsageNotObject),
                "{projection:?} {body}"
            );
            assert!(run.logged(), "{projection:?} {body} is logged");
        }
    }
    for (body, field) in [
        (
            r#"{"generation": "x", "usage": {"prompt_tokens": "5"}}"#,
            "prompt_tokens",
        ),
        (
            r#"{"generation": "x", "usage": {"completion_tokens": true}}"#,
            "completion_tokens",
        ),
        (
            r#"{"generation": "x", "usage": {"total_tokens": 5.5}}"#,
            "total_tokens",
        ),
        (
            r#"{"generation": "x", "usage": {"prompt_tokens": 9223372036854775808}}"#,
            "prompt_tokens",
        ),
    ] {
        for projection in BOTH {
            let run = qwen(&case, projection, body);
            assert!(
                matches!(run.error(), QwenReplyError::UsageCount { name } if *name == field),
                "{projection:?} {body}"
            );
        }
    }
}

fn json_and_shape_errors() {
    let case = fixture();
    for body in ["not json", "{", r#"{"generation": "x""#] {
        let run = qwen(&case, QwenProjection::Qwen2, body);
        assert!(
            matches!(run.error(), QwenReplyError::Decode(_)),
            "body {body}"
        );
        assert!(run.logged(), "body {body} is logged");
    }
    for body in ["[]", r#""x""#, "5", "null"] {
        let run = qwen(&case, QwenProjection::Qwen2, body);
        assert!(
            matches!(run.error(), QwenReplyError::NotObject),
            "body {body}"
        );
    }
    let qwen3_null = qwen(&case, QwenProjection::Qwen3, r#"{"generation": null}"#);
    assert!(matches!(
        qwen3_null.error(),
        QwenReplyError::NotText { field } if *field == "generation"
    ));
    let qwen2_number = qwen(&case, QwenProjection::Qwen2, r#"{"generation": 5}"#);
    assert!(matches!(
        qwen2_number.error(),
        QwenReplyError::NotText { field } if *field == "generation"
    ));

    let logged = qwen(&case, QwenProjection::Qwen3, "not json");
    assert_eq!(
        logged.logged_response(),
        &Json::Text(String::from("not json")),
        "the raw body is logged as sent"
    );
    assert_eq!(
        logged.logged_input(),
        &case.messages,
        "the source messages are logged"
    );
    assert_eq!(
        logged.logged_error(),
        "the response body is not valid JSON",
        "the error text is logged"
    );
    assert_eq!(
        logged.details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

fn successful_replies_log_nothing() {
    let case = fixture();
    for projection in BOTH {
        let run = qwen(&case, projection, r#"{"generation": "x"}"#);
        assert!(!run.logged(), "{projection:?} logs no post_call");
        assert!(!run.details.contains_key("additional_args"));
        assert!(!run.details.contains_key("original_response"));
    }
}
