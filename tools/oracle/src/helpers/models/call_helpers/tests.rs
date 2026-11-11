#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_context::request_records::record::{HandleKind, InternalHandle, RequestRecord};

fn span_of<'record>(record: &'record RequestRecord, bucket_name: &str) -> &'record RequestRecord {
    let RequestRecord::Object(fields) = record else {
        panic!("the record is not an object");
    };
    let bucket = fields.get(bucket_name).unwrap_or_else(|| {
        panic!("the record has no {bucket_name} bucket");
    });
    let RequestRecord::Object(bucket_fields) = bucket else {
        panic!("{bucket_name} is not an object");
    };
    bucket_fields
        .get("litellm_parent_otel_span")
        .unwrap_or_else(|| panic!("{bucket_name} has no litellm_parent_otel_span"))
}

fn record_with_two_spans() -> (RequestRecord, RequestRecord, RequestRecord) {
    let span_a = RequestRecord::Handle(InternalHandle {
        kind: HandleKind::Opaque,
        identity: 101,
    });
    let span_b = RequestRecord::Handle(InternalHandle {
        kind: HandleKind::Opaque,
        identity: 202,
    });
    let record = RequestRecord::Object(BTreeMap::from([
        (
            "metadata".to_owned(),
            RequestRecord::Object(BTreeMap::from([(
                "litellm_parent_otel_span".to_owned(),
                span_a.clone(),
            )])),
        ),
        (
            "litellm_metadata".to_owned(),
            RequestRecord::Object(BTreeMap::from([(
                "litellm_parent_otel_span".to_owned(),
                span_b.clone(),
            )])),
        ),
    ]));
    (record, span_a, span_b)
}

#[test]
fn the_snapshot_keeps_both_spans_and_leaves_the_source_unchanged() {
    let (source, span_a, span_b) = record_with_two_spans();
    // the B2204 independent snapshot is a plain clone; each copied bucket keeps its own span
    let copy = source.clone();
    assert_eq!(span_of(&copy, "metadata"), &span_a);
    assert_eq!(span_of(&copy, "litellm_metadata"), &span_b);
    let RequestRecord::Object(mut copy_fields) = copy else {
        panic!("the snapshot is not an object");
    };
    let replacement = RequestRecord::Handle(InternalHandle {
        kind: HandleKind::Opaque,
        identity: 909,
    });
    let RequestRecord::Object(copied_metadata) =
        copy_fields.get_mut("metadata").expect("metadata bucket")
    else {
        panic!("the copied metadata is not an object");
    };
    copied_metadata.insert("litellm_parent_otel_span".to_owned(), replacement);
    assert_eq!(span_of(&source, "metadata"), &span_a);
    assert_eq!(span_of(&source, "litellm_metadata"), &span_b);
}
