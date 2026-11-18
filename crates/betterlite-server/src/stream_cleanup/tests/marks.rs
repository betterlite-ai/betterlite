use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::super::marks::{
    apply_disconnect_marks, disconnect_error_info, mark_details_disconnected,
    mark_request_disconnected,
};
use super::support::{
    Ending, FakeLogging, FakeSlots, cleanup_outcome, disconnect_cleanup, expected_error_info,
    object, text,
};

#[test]
fn disconnect_error_info_is_a_fresh_499_object() {
    let mut first = disconnect_error_info();
    assert_eq!(Json::Object(first.clone()), expected_error_info());
    first.insert("extra".to_owned(), Json::Bool(true));
    assert_eq!(Json::Object(disconnect_error_info()), expected_error_info());
}

#[test]
fn apply_disconnect_marks_keeps_other_keys_and_replaces_information() {
    let mut target = object(&[
        ("user", text("x")),
        ("client_disconnected", Json::Bool(false)),
        (
            "error_information",
            Json::Object(object(&[("error_code", text("400"))])),
        ),
    ]);
    apply_disconnect_marks(&mut target);
    assert_eq!(
        target,
        object(&[
            ("user", text("x")),
            ("client_disconnected", Json::Bool(true)),
            ("error_information", expected_error_info()),
        ])
    );
}

#[test]
fn mark_request_disconnected_creates_missing_containers() {
    let mut request_data = object(&[("litellm_call_id", text("call-1"))]);
    mark_request_disconnected(&mut request_data);
    let marked = object(&[
        ("client_disconnected", Json::Bool(true)),
        ("error_information", expected_error_info()),
    ]);
    assert_eq!(
        request_data,
        object(&[
            ("litellm_call_id", text("call-1")),
            ("metadata", Json::Object(marked.clone())),
            (
                "litellm_params",
                Json::Object(object(&[("metadata", Json::Object(marked))])),
            ),
        ])
    );
}

#[test]
fn mark_request_disconnected_marks_existing_objects() {
    let mut request_data = object(&[
        ("metadata", Json::Object(object(&[("user", text("x"))]))),
        (
            "litellm_params",
            Json::Object(object(&[
                ("model", text("gpt-4o")),
                ("metadata", Json::Object(object(&[("team", text("t"))]))),
            ])),
        ),
    ]);
    mark_request_disconnected(&mut request_data);
    assert_eq!(
        request_data,
        object(&[
            (
                "metadata",
                Json::Object(object(&[
                    ("user", text("x")),
                    ("client_disconnected", Json::Bool(true)),
                    ("error_information", expected_error_info()),
                ])),
            ),
            (
                "litellm_params",
                Json::Object(object(&[
                    ("model", text("gpt-4o")),
                    (
                        "metadata",
                        Json::Object(object(&[
                            ("team", text("t")),
                            ("client_disconnected", Json::Bool(true)),
                            ("error_information", expected_error_info()),
                        ])),
                    ),
                ])),
            ),
        ])
    );
}

#[test]
fn mark_request_disconnected_replaces_null_containers() {
    let mut request_data = object(&[("metadata", Json::Null), ("litellm_params", Json::Null)]);
    mark_request_disconnected(&mut request_data);
    let marked = object(&[
        ("client_disconnected", Json::Bool(true)),
        ("error_information", expected_error_info()),
    ]);
    assert_eq!(
        request_data,
        object(&[
            ("metadata", Json::Object(marked.clone())),
            (
                "litellm_params",
                Json::Object(object(&[("metadata", Json::Object(marked))])),
            ),
        ])
    );
}

#[test]
fn mark_request_disconnected_keeps_non_object_metadata() {
    let original = object(&[
        ("metadata", text("raw")),
        (
            "litellm_params",
            Json::Object(object(&[("metadata", text("raw-inner"))])),
        ),
    ]);
    let mut request_data = original.clone();
    mark_request_disconnected(&mut request_data);
    assert_eq!(request_data, original);
}

#[test]
fn mark_details_disconnected_marks_both_metadata_copies() {
    let mut details = object(&[
        (
            "litellm_params",
            Json::Object(object(&[(
                "metadata",
                Json::Object(object(&[("kept", text("v"))])),
            )])),
        ),
        ("metadata", Json::Object(object(&[("kept", text("w"))]))),
    ]);
    mark_details_disconnected(&mut details);
    assert_eq!(
        details,
        object(&[
            (
                "litellm_params",
                Json::Object(object(&[(
                    "metadata",
                    Json::Object(object(&[
                        ("kept", text("v")),
                        ("client_disconnected", Json::Bool(true)),
                        ("error_information", expected_error_info()),
                    ])),
                )])),
            ),
            (
                "metadata",
                Json::Object(object(&[
                    ("kept", text("w")),
                    ("client_disconnected", Json::Bool(true)),
                    ("error_information", expected_error_info()),
                ])),
            ),
        ])
    );
}

#[test]
fn mark_details_disconnected_creates_missing_objects() {
    let mut details = object(&[]);
    mark_details_disconnected(&mut details);
    let marked = object(&[
        ("client_disconnected", Json::Bool(true)),
        ("error_information", expected_error_info()),
    ]);
    assert_eq!(
        details,
        object(&[
            (
                "litellm_params",
                Json::Object(object(&[("metadata", Json::Object(marked.clone()))])),
            ),
            ("metadata", Json::Object(marked)),
        ])
    );
}

#[test]
fn mark_details_disconnected_replaces_null_containers() {
    let mut details = object(&[("litellm_params", Json::Null), ("metadata", Json::Null)]);
    mark_details_disconnected(&mut details);
    let marked = object(&[
        ("client_disconnected", Json::Bool(true)),
        ("error_information", expected_error_info()),
    ]);
    assert_eq!(
        details,
        object(&[
            (
                "litellm_params",
                Json::Object(object(&[("metadata", Json::Object(marked.clone()))])),
            ),
            ("metadata", Json::Object(marked)),
        ])
    );
}

#[tokio::test]
async fn run_cleanup_marks_the_request_and_the_logging_details_on_disconnect() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::default());
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    let mut outcome = cleanup_outcome(Ending::Disconnected);
    outcome.request_data.insert(
        "metadata".to_owned(),
        Json::Object(object(&[("user", text("x"))])),
    );
    cleanup.run_cleanup(outcome).await.unwrap();

    let marks = object(&[
        ("client_disconnected", Json::Bool(true)),
        ("error_information", expected_error_info()),
    ]);
    assert_eq!(
        logging.details(),
        Some(object(&[
            ("metadata", Json::Object(marks.clone())),
            (
                "litellm_params",
                Json::Object(object(&[("metadata", Json::Object(marks.clone()))])),
            ),
        ]))
    );
    let fired_with = logging.fired_with().expect("fired request data");
    assert_eq!(
        fired_with,
        object(&[
            ("litellm_call_id", text("call-1")),
            (
                "metadata",
                Json::Object(object(&[
                    ("client_disconnected", Json::Bool(true)),
                    ("error_information", expected_error_info()),
                    ("user", text("x")),
                ])),
            ),
            (
                "litellm_params",
                Json::Object(object(&[("metadata", Json::Object(marks))])),
            ),
        ])
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
}

#[tokio::test]
async fn run_cleanup_leaves_a_connected_request_unmarked() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::default());
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Completed))
        .await
        .unwrap();
    assert!(logging.events().is_empty());
    assert_eq!(logging.details(), Some(object(&[])));
    assert_eq!(logging.fired(), 0);
    assert!(slots.keys().is_empty());
}
