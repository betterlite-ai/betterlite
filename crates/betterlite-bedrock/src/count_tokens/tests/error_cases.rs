use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::validate::validate_count_tokens_request;

use super::support::{MODEL, request};

pub(super) fn count_model_missing() {
    let absent = request(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    assert_eq!(
        validate_count_tokens_request(&absent),
        Err(CountTokensError::ModelRequired)
    );
    let null = request(r#"{"messages": [{"role": "user", "content": "hi"}], "model": null}"#);
    assert_eq!(
        validate_count_tokens_request(&null),
        Err(CountTokensError::ModelRequired)
    );
    let false_model =
        request(r#"{"messages": [{"role": "user", "content": "hi"}], "model": false}"#);
    assert_eq!(
        validate_count_tokens_request(&false_model),
        Err(CountTokensError::ModelRequired)
    );
    let empty = request(r#"{"messages": [{"role": "user", "content": "hi"}], "model": ""}"#);
    assert_eq!(
        validate_count_tokens_request(&empty),
        Err(CountTokensError::ModelRequired)
    );
    let zero = request(r#"{"messages": [{"role": "user", "content": "hi"}], "model": 0}"#);
    assert_eq!(
        validate_count_tokens_request(&zero),
        Err(CountTokensError::ModelRequired)
    );
    let empty_list = request(r#"{"messages": [{"role": "user", "content": "hi"}], "model": []}"#);
    assert_eq!(
        validate_count_tokens_request(&empty_list),
        Err(CountTokensError::ModelRequired)
    );
    let empty_object = request(r#"{"messages": [{"role": "user", "content": "hi"}], "model": {}}"#);
    assert_eq!(
        validate_count_tokens_request(&empty_object),
        Err(CountTokensError::ModelRequired)
    );
}

pub(super) fn count_messages_empty() {
    let empty = request(&format!(r#"{{"messages": [], "model": "{MODEL}"}}"#));
    assert_eq!(
        validate_count_tokens_request(&empty),
        Err(CountTokensError::MessagesRequired)
    );
}

pub(super) fn count_message_not_object() {
    let first = request(&format!(r#"{{"messages": [42], "model": "{MODEL}"}}"#));
    assert_eq!(
        validate_count_tokens_request(&first),
        Err(CountTokensError::MessageNotObject { index: 0 })
    );
    let second = request(&format!(
        r#"{{"messages": [{{"role": "user", "content": "hi"}}, 7], "model": "{MODEL}"}}"#
    ));
    assert_eq!(
        validate_count_tokens_request(&second),
        Err(CountTokensError::MessageNotObject { index: 1 })
    );
}

pub(super) fn count_message_role_missing() {
    let first = request(&format!(
        r#"{{"messages": [{{"content": "hi"}}], "model": "{MODEL}"}}"#
    ));
    assert_eq!(
        validate_count_tokens_request(&first),
        Err(CountTokensError::MessageFieldMissing {
            index: 0,
            field: "role",
        })
    );
    let second = request(&format!(
        r#"{{"messages": [{{"role": "user", "content": "hi"}}, {{"content": "x"}}], "model": "{MODEL}"}}"#
    ));
    assert_eq!(
        validate_count_tokens_request(&second),
        Err(CountTokensError::MessageFieldMissing {
            index: 1,
            field: "role",
        })
    );
}

pub(super) fn count_message_content_missing() {
    let first = request(&format!(
        r#"{{"messages": [{{"role": "user"}}], "model": "{MODEL}"}}"#
    ));
    assert_eq!(
        validate_count_tokens_request(&first),
        Err(CountTokensError::MessageFieldMissing {
            index: 0,
            field: "content",
        })
    );
    let second = request(&format!(
        r#"{{"messages": [{{"role": "user", "content": "hi"}}, {{"role": "user"}}], "model": "{MODEL}"}}"#
    ));
    assert_eq!(
        validate_count_tokens_request(&second),
        Err(CountTokensError::MessageFieldMissing {
            index: 1,
            field: "content",
        })
    );
}

pub(super) fn count_invoke_model_only() {
    let only_model = request(&format!(r#"{{"model": "{MODEL}"}}"#));
    assert_eq!(
        validate_count_tokens_request(&only_model),
        Err(CountTokensError::ContentRequired)
    );
    let with_extra = request(&format!(r#"{{"model": "{MODEL}", "prompt": null}}"#));
    assert_eq!(validate_count_tokens_request(&with_extra), Ok(()));
}
