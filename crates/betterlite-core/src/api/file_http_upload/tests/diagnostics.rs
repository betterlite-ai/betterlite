use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::file_http_upload::plan::UploadPlan;
use crate::api::file_http_upload::service::Mode;

use super::{API_BASE, Harness, create_call, field, params, text};

// an unsupported request is logged before it fails, and its top-level headers are masked like any request
#[tokio::test]
async fn upload_diagnostics_mask_a_top_level_headers_dict() {
    let plan = UploadPlan::Unsupported(Box::new(Json::Object(IndexMap::from([
        (
            String::from("headers"),
            Json::Object(IndexMap::from([
                (String::from("Authorization"), text("Bearer sk-secret")),
                (String::from("x-trace"), text("trace-1")),
            ])),
        ),
        (String::from("other"), text("kept")),
    ]))));
    let mut harness = Harness::new(plan, API_BASE, Vec::new());
    let error = harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unsupported transformed_request type: <class 'dict'>"
    );
    let details = harness.details();
    let body = field(field(&details, "additional_args"), "complete_input_dict");
    let headers = field(body, "headers");
    assert_eq!(field(headers, "Authorization"), &text("Be****et"));
    assert_eq!(field(headers, "x-trace"), &text("trace-1"));
    assert_eq!(field(body, "other"), &text("kept"));
}

// masking leaves a request alone when its headers member is not a dict, and when the request is no dict
#[tokio::test]
async fn upload_diagnostics_leave_requests_without_a_headers_dict_alone() {
    let plan = UploadPlan::Unsupported(Box::new(Json::Object(IndexMap::from([(
        String::from("headers"),
        text("not a dict"),
    )]))));
    let mut harness = Harness::new(plan, API_BASE, Vec::new());
    harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap_err();
    let details = harness.details();
    let body = field(field(&details, "additional_args"), "complete_input_dict");
    assert_eq!(field(body, "headers"), &text("not a dict"));

    let plan = UploadPlan::Unsupported(Box::new(text("plain body")));
    let mut harness = Harness::new(plan, API_BASE, Vec::new());
    harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap_err();
    let details = harness.details();
    let body = field(field(&details, "additional_args"), "complete_input_dict");
    assert_eq!(body, &text("plain body"));
}
