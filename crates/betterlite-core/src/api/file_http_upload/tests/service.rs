use std::collections::BTreeMap;

use betterlite_anthropic::files::request::create_file_parts;
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{UploadData, UploadUrlLocation};
use betterlite_openai::files::records::FileCreate;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::Method;
use rstest::rstest;

use crate::api::file_http_upload::config::{
    FileParams, FileResponse, FileUploadConfig, Headers, UploadParams,
};
use crate::api::file_http_upload::error::{TransformFailure, UploadError, UrlFailure};
use crate::api::file_http_upload::plan::{MultipartPlan, PresignedPlan, UploadPlan};
use crate::api::file_http_upload::service::Mode;
use crate::api::vector_store_http_write::error::BaseLlmException;

use super::{
    API_BASE, Config, FakeTransport, Harness, NOW, UPLOAD_URL, Wire, answer, create_call, field,
    manus_config, manus_plan, params, status, text, text_plan, two_step_plan,
};

// A provider config that keeps the trait's own file_upload_http_method, the way a plain provider does
struct PlainConfig {
    inner: Config,
}

impl FileUploadConfig for PlainConfig {
    fn validate_environment(&self, headers: Headers, params: &UploadParams<'_>) -> Headers {
        self.inner.validate_environment(headers, params)
    }

    fn get_complete_file_url(
        &self,
        params: &UploadParams<'_>,
        data: &FileCreate,
    ) -> Result<Option<String>, UrlFailure> {
        self.inner.get_complete_file_url(params, data)
    }

    fn transform_create_file_request(
        &self,
        data: &FileCreate,
        params: &UploadParams<'_>,
    ) -> Result<UploadPlan, TransformFailure> {
        self.inner.transform_create_file_request(data, params)
    }

    fn transform_create_file_response(
        &self,
        response: &FileResponse,
        litellm_params: &FileParams,
    ) -> Result<Json, BaseLlmException> {
        self.inner
            .transform_create_file_response(response, litellm_params)
    }
}

// the trait default is POST, LiteLLM's BaseFilesConfig.file_upload_http_method
#[test]
fn file_upload_http_method_defaults_to_post() {
    let config = PlainConfig {
        inner: Config::new(text_plan(), API_BASE),
    };
    assert_eq!(config.file_upload_http_method(), Method::POST);
}

// create_file uses a supplied client only when its class matches; the other class gets a fresh one
#[tokio::test]
async fn supplied_client_is_used_only_for_its_own_mode() {
    let supplied = FakeTransport {
        wire: Wire::with(vec![answer(200, &[], b"{}"), answer(200, &[], b"{}")]),
    };
    let mut harness = Harness::new(text_plan(), API_BASE, vec![answer(200, &[], b"{}")])
        .with_supplied(Mode::Sync, supplied.clone());
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(supplied.wire.seen().len(), 1);
    assert!(harness.seen().is_empty());

    let async_supplied = FakeTransport {
        wire: Wire::with(vec![answer(200, &[], b"{}"), answer(200, &[], b"{}")]),
    };
    let mut async_harness = Harness::new(text_plan(), API_BASE, vec![answer(200, &[], b"{}")])
        .with_supplied(Mode::Asynchronous, async_supplied.clone());
    async_harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert!(async_supplied.wire.seen().is_empty());
    assert_eq!(async_harness.seen().len(), 1);
}

// a multipart plan always posts, as LiteLLM's files= argument is a POST whatever the config asks
#[tokio::test]
async fn multipart_upload_is_post_even_when_the_config_asks_put() {
    let file = FileData::Tuple3 {
        filename: Some(String::from("rows.jsonl")),
        content: FileBody::Bytes(b"{\"row\":1}\n".to_vec()),
        content_type: Some(String::from("application/jsonl")),
    };
    let parts = create_file_parts(Some(&file), Some("batch"), NOW).expect("anthropic parts");
    let plan = UploadPlan::Multipart(Box::new(MultipartPlan { parts }));
    let config = Config::with_method(plan, API_BASE, Method::PUT);
    let mut harness = Harness::build(config, vec![answer(200, &[], b"{}")]);
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(harness.seen()[0].method, Method::POST);
}

// a method string that is not an HTTP token fails naming it, where LiteLLM raises AttributeError
#[tokio::test]
async fn two_step_upload_invalid_method_names_the_method() {
    let mut config = manus_config();
    config.upload_request.method = String::from("GE T");
    let mut harness = Harness::new(
        two_step_plan(config, Some(UploadUrlLocation::Body)),
        API_BASE,
        vec![answer(
            200,
            &[],
            b"{\"upload_url\":\"http://uploads.example.test/manus/1\"}",
        )],
    );
    let error = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    let UploadError::Provider(provider) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the upload method GE T is not an HTTP method"
    );
    // the initial request went out and the failure stopped the second one
    assert_eq!(harness.seen().len(), 1);
}

// A provider error as the service reports it, failing the test when another error came back
fn provider_failure(error: UploadError) -> BaseLlmException {
    let UploadError::Provider(provider) = error else {
        panic!("expected a provider error");
    };
    provider
}

// a 4xx initial answer is the provider's status, raised before the location decode, in both modes
#[rstest]
#[case::sync(Mode::Sync)]
#[case::asynchronous(Mode::Asynchronous)]
#[tokio::test]
async fn two_step_initial_status_failure_keeps_the_provider_status(#[case] mode: Mode) {
    let mut harness = Harness::new(
        manus_plan(),
        API_BASE,
        vec![answer(
            429,
            &[("content-type", "application/json")],
            b"{\"error\":\"slow down\"}",
        )],
    );
    let error = provider_failure(
        harness
            .run(mode, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "{\"error\":\"slow down\"}");
    assert_eq!(
        error.headers.get("content-type"),
        Some(&text("application/json"))
    );
    assert_eq!(harness.seen().len(), 1);
    assert!(
        !harness
            .config
            .calls()
            .contains(&"transform_create_file_response")
    );
}

// a redirect is not an error for httpx, so the two-step still reads the upload URL from it
#[tokio::test]
async fn two_step_redirect_answer_still_reads_the_location() {
    let mut harness = Harness::new(
        manus_plan(),
        API_BASE,
        vec![
            answer(
                302,
                &[("content-type", "application/json")],
                b"{\"upload_url\":\"http://uploads.example.test/manus/2\"}",
            ),
            answer(200, &[], b"{\"id\":\"file-1\"}"),
        ],
    );
    let result = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(harness.seen()[1].url, "http://uploads.example.test/manus/2");
    assert_eq!(field(&result, "status"), &status(200));
}

// a failed post on the second step keeps the provider's status instead of a synthesized 500
#[rstest]
#[case::sync(Mode::Sync)]
#[case::asynchronous(Mode::Asynchronous)]
#[tokio::test]
async fn two_step_second_post_status_failure_keeps_the_provider_status(#[case] mode: Mode) {
    let mut config = manus_config();
    config.upload_request.method = String::from("POST");
    let mut harness = Harness::new(
        two_step_plan(config, Some(UploadUrlLocation::Body)),
        API_BASE,
        vec![
            answer(
                200,
                &[],
                b"{\"upload_url\":\"http://uploads.example.test/manus/1\"}",
            ),
            answer(503, &[], b"upload unavailable"),
        ],
    );
    let error = provider_failure(
        harness
            .run(mode, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (503, "upload unavailable")
    );
    assert_eq!(harness.seen().len(), 2);
    assert!(
        !harness
            .config
            .calls()
            .contains(&"transform_create_file_response")
    );
}

// a put second step raises only in async mode; the sync handler hands the answer to the transform
#[tokio::test]
async fn two_step_put_failure_raises_only_in_async_mode() {
    let answers = || {
        vec![
            answer(
                200,
                &[],
                b"{\"upload_url\":\"http://uploads.example.test/manus/1\"}",
            ),
            answer(400, &[], b"bad upload"),
        ]
    };
    let mut asynchronous = Harness::new(manus_plan(), API_BASE, answers());
    let error = provider_failure(
        asynchronous
            .run(Mode::Asynchronous, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (400, "bad upload")
    );

    let mut sync = Harness::new(manus_plan(), API_BASE, answers());
    let result = sync
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(field(&result, "status"), &status(400));
    let saved = field(field(&result, "params"), "initial_file_response");
    assert_eq!(
        field(saved, "upload_url"),
        &text("http://uploads.example.test/manus/1")
    );
}

// a presigned put answers the same way: only the async handler raises on a 4xx/5xx
#[tokio::test]
async fn presigned_status_failure_raises_only_in_async_mode() {
    let plan = || {
        UploadPlan::Presigned(Box::new(PresignedPlan {
            method: String::from("PUT"),
            url: String::from(UPLOAD_URL),
            headers: BTreeMap::new(),
            data: UploadData::Bytes(b"pdf".to_vec()),
        }))
    };
    let mut asynchronous = Harness::new(plan(), API_BASE, vec![answer(503, &[], b"nope")]);
    let error = provider_failure(
        asynchronous
            .run(Mode::Asynchronous, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!((error.status_code, error.message.as_str()), (503, "nope"));

    let mut sync = Harness::new(plan(), API_BASE, vec![answer(503, &[], b"nope")]);
    let result = sync
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(field(&result, "status"), &status(503));
    assert!(
        sync.config
            .calls()
            .contains(&"transform_create_file_response")
    );
}

// a text post raises on a 4xx in both modes, before the transform
#[rstest]
#[case::sync(Mode::Sync)]
#[case::asynchronous(Mode::Asynchronous)]
#[tokio::test]
async fn traditional_post_status_failure_raises_in_both_modes(#[case] mode: Mode) {
    let mut harness = Harness::new(text_plan(), API_BASE, vec![answer(400, &[], b"rejected")]);
    let error = provider_failure(
        harness
            .run(mode, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (400, "rejected")
    );
    assert_eq!(harness.seen().len(), 1);
    assert!(
        !harness
            .config
            .calls()
            .contains(&"transform_create_file_response")
    );
}

// a text put is unjudged in sync mode and raises in async mode
#[tokio::test]
async fn traditional_put_status_failure_raises_only_in_async_mode() {
    let config = || Config::with_method(text_plan(), API_BASE, Method::PUT);
    let mut asynchronous = Harness::build(config(), vec![answer(500, &[], b"boom")]);
    let error = provider_failure(
        asynchronous
            .run(Mode::Asynchronous, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!((error.status_code, error.message.as_str()), (500, "boom"));

    let mut sync = Harness::build(config(), vec![answer(500, &[], b"boom")]);
    let result = sync
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(field(&result, "status"), &status(500));
}

// a multipart upload posts whatever the config asks, so its 4xx raises in sync mode too
#[rstest]
#[case::sync(Mode::Sync)]
#[case::asynchronous(Mode::Asynchronous)]
#[tokio::test]
async fn multipart_status_failure_raises_in_sync_mode(#[case] mode: Mode) {
    let file = FileData::Tuple3 {
        filename: Some(String::from("rows.jsonl")),
        content: FileBody::Bytes(b"{\"row\":1}\n".to_vec()),
        content_type: Some(String::from("application/jsonl")),
    };
    let parts = create_file_parts(Some(&file), Some("batch"), NOW).expect("anthropic parts");
    let plan = UploadPlan::Multipart(Box::new(MultipartPlan { parts }));
    let mut harness = Harness::build(
        Config::with_method(plan, API_BASE, Method::PUT),
        vec![answer(400, &[], b"form rejected")],
    );
    let error = provider_failure(
        harness
            .run(mode, &create_call(), &params())
            .await
            .unwrap_err(),
    );
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (400, "form rejected")
    );
    assert_eq!(harness.seen()[0].method, Method::POST);
}

// the body location's default key is upload_url when the plan names none
#[tokio::test]
async fn two_step_upload_default_url_key() {
    let mut config = manus_config();
    config.upload_url_key = None;
    let mut harness = Harness::new(
        two_step_plan(config, Some(UploadUrlLocation::Body)),
        API_BASE,
        vec![
            answer(
                200,
                &[],
                b"{\"upload_url\":\"http://uploads.example.test/default/1\"}",
            ),
            answer(200, &[], b"{}"),
        ],
    );
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(
        harness.seen()[1].url,
        "http://uploads.example.test/default/1"
    );
}
