use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::file_http_upload::plan::{MultipartPlan, UploadPlan};
use crate::api::file_http_upload::service::Mode;

use super::{API_BASE, Harness, answer, bytes_json, create_call, field, manus_plan, params, text};

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

// a two-step plan logs both requests as LiteLLM would build them, with the location and key it read
#[tokio::test]
async fn upload_diagnostics_logs_the_two_step_requests() {
    let mut harness = Harness::new(
        manus_plan(),
        API_BASE,
        vec![
            answer(
                200,
                &[],
                b"{\"upload_url\":\"http://uploads.example.test/manus/1\"}",
            ),
            answer(200, &[], b"{}"),
        ],
    );
    harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap();
    let details = harness.details();
    let body = field(field(&details, "additional_args"), "complete_input_dict");
    let initial = field(body, "initial_request");
    assert_eq!(field(initial, "method"), &text("POST"));
    assert_eq!(field(initial, "url"), &text(API_BASE));
    assert_eq!(
        field(field(initial, "headers"), "API_KEY"),
        &text("sk-manus")
    );
    assert_eq!(
        field(field(initial, "data"), "filename"),
        &text("rows.jsonl")
    );
    let upload = field(body, "upload_request");
    assert_eq!(field(upload, "method"), &text("PUT"));
    assert_eq!(field(upload, "url"), &text(""));
    assert_eq!(field(upload, "data"), &bytes_json(b"{\"row\":1}\n"));
    assert_eq!(field(body, "upload_url_location"), &text("body"));
    assert_eq!(field(body, "upload_url_key"), &text("upload_url"));
}

// a multipart plan logs the files= mapping as httpx would read it: filename, bytes, then the type
#[tokio::test]
async fn upload_diagnostics_logs_the_multipart_form() {
    let plan = UploadPlan::Multipart(Box::new(MultipartPlan {
        parts: vec![MultipartPart {
            name: String::from("file"),
            file_name: Some(String::from("rows.jsonl")),
            content_type: Some(String::from("application/jsonl")),
            data: b"{\"row\":1}\n".to_vec(),
        }],
    }));
    let mut harness = Harness::new(plan, API_BASE, vec![answer(200, &[], b"{}")]);
    harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap();
    let details = harness.details();
    let body = field(field(&details, "additional_args"), "complete_input_dict");
    let expected = Json::Object(IndexMap::from([(
        String::from("file"),
        Json::List(vec![
            text("rows.jsonl"),
            bytes_json(b"{\"row\":1}\n"),
            text("application/jsonl"),
        ]),
    )]));
    assert_eq!(body, &expected);
}
