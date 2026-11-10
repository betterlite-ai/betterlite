#![cfg(test)]

use super::support_tests::{empty_request, number, object, with_params};
use crate::cache_key::request::KeyParam;
use crate::cache_key::value::KeyValue;
use crate::envelope::lookup::{
    CacheLookup, SEMANTIC_SIMILARITY, stamp_semantic_similarity, update_metadata,
};

#[test]
fn projection_keeps_only_messages_input_and_metadata() {
    let mut request = with_params(vec![
        KeyParam {
            name: "model".to_owned(),
            value: KeyValue::String("gpt-4".to_owned()),
        },
        KeyParam {
            name: "messages".to_owned(),
            value: KeyValue::String("hi".to_owned()),
        },
        KeyParam {
            name: "input".to_owned(),
            value: KeyValue::String("text".to_owned()),
        },
        KeyParam {
            name: "temperature".to_owned(),
            value: number(0.7),
        },
    ]);
    request.key.metadata = Some(object(&[(
        "user_api_key",
        KeyValue::String("key-hash".to_owned()),
    )]));
    assert_eq!(
        CacheLookup::project(&request.key),
        CacheLookup {
            messages: Some(KeyValue::String("hi".to_owned())),
            input: Some(KeyValue::String("text".to_owned())),
            metadata: Some(object(&[(
                "user_api_key",
                KeyValue::String("key-hash".to_owned())
            )])),
        }
    );
}

#[test]
fn projection_keeps_a_null_prompt_value() {
    let request = with_params(vec![KeyParam {
        name: "messages".to_owned(),
        value: KeyValue::Null,
    }]);
    assert_eq!(
        CacheLookup::project(&request.key),
        CacheLookup {
            messages: Some(KeyValue::Null),
            input: None,
            metadata: None,
        }
    );
}

#[test]
fn projection_copies_the_metadata() {
    let mut request = empty_request();
    request.key.metadata = Some(object(&[("a", KeyValue::String("b".to_owned()))]));
    let mut lookup = CacheLookup::project(&request.key);
    lookup
        .metadata
        .as_mut()
        .expect("lookup metadata")
        .insert("c".to_owned(), KeyValue::String("d".to_owned()));
    assert_eq!(
        request.key.metadata,
        Some(object(&[("a", KeyValue::String("b".to_owned()))]))
    );
}

#[test]
fn metadata_update_copies_semantic_similarity() {
    let mut request = empty_request();
    request.key.metadata = Some(object(&[("a", KeyValue::String("b".to_owned()))]));
    let lookup = CacheLookup {
        messages: None,
        input: None,
        metadata: Some(object(&[
            ("a", KeyValue::String("b".to_owned())),
            (SEMANTIC_SIMILARITY, number(0.5)),
        ])),
    };
    update_metadata(&mut request.key, &lookup);
    assert_eq!(
        request.key.metadata,
        Some(object(&[
            ("a", KeyValue::String("b".to_owned())),
            (SEMANTIC_SIMILARITY, number(0.5)),
        ]))
    );
}

#[test]
fn metadata_update_needs_request_metadata() {
    let mut request = empty_request();
    let lookup = CacheLookup {
        messages: None,
        input: None,
        metadata: Some(object(&[(SEMANTIC_SIMILARITY, number(0.5))])),
    };
    update_metadata(&mut request.key, &lookup);
    assert_eq!(request.key.metadata, None);
}

#[test]
fn metadata_update_needs_lookup_metadata() {
    let mut request = empty_request();
    request.key.metadata = Some(object(&[("a", KeyValue::String("b".to_owned()))]));
    let lookup = CacheLookup {
        messages: None,
        input: None,
        metadata: None,
    };
    update_metadata(&mut request.key, &lookup);
    assert_eq!(
        request.key.metadata,
        Some(object(&[("a", KeyValue::String("b".to_owned()))]))
    );
}

#[test]
fn metadata_update_needs_the_similarity_key() {
    let mut request = empty_request();
    request.key.metadata = Some(object(&[("a", KeyValue::String("b".to_owned()))]));
    let lookup = CacheLookup {
        messages: None,
        input: None,
        metadata: Some(object(&[("a", KeyValue::String("b".to_owned()))])),
    };
    update_metadata(&mut request.key, &lookup);
    assert_eq!(
        request.key.metadata,
        Some(object(&[("a", KeyValue::String("b".to_owned()))]))
    );
}

#[test]
fn stamp_semantic_similarity_writes_a_number() {
    let mut request = empty_request();
    request.key.metadata = Some(object(&[("a", KeyValue::String("b".to_owned()))]));
    stamp_semantic_similarity(&mut request.key, Some(0.25_f64));
    assert_eq!(
        request.key.metadata,
        Some(object(&[
            ("a", KeyValue::String("b".to_owned())),
            (SEMANTIC_SIMILARITY, number(0.25))
        ]))
    );
}

#[test]
fn stamp_semantic_similarity_skips_an_absent_value() {
    let mut request = empty_request();
    request.key.metadata = Some(object(&[("a", KeyValue::String("b".to_owned()))]));
    stamp_semantic_similarity(&mut request.key, None);
    assert_eq!(
        request.key.metadata,
        Some(object(&[("a", KeyValue::String("b".to_owned()))]))
    );
}

#[test]
fn stamp_semantic_similarity_needs_request_metadata() {
    let mut request = empty_request();
    stamp_semantic_similarity(&mut request.key, Some(0.25_f64));
    assert_eq!(request.key.metadata, None);
}
