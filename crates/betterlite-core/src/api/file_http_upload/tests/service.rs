use betterlite_anthropic::files::request::create_file_parts;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileCreate;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::Method;

use crate::api::file_http_upload::config::{
    FileParams, FileResponse, FileUploadConfig, Headers, UploadParams,
};
use crate::api::file_http_upload::error::{TransformFailure, UploadError, UrlFailure};
use crate::api::file_http_upload::plan::{MultipartPlan, UploadPlan};
use crate::api::file_http_upload::service::Mode;
use crate::api::vector_store_http_write::error::BaseLlmException;

use super::{
    API_BASE, Config, FakeTransport, Harness, NOW, Wire, answer, create_call, manus_config, params,
    text_plan, two_step_plan,
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
        params: &FileParams,
    ) -> Result<Json, BaseLlmException> {
        self.inner.transform_create_file_response(response, params)
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

    let supplied = FakeTransport {
        wire: Wire::with(vec![answer(200, &[], b"{}"), answer(200, &[], b"{}")]),
    };
    let mut harness = Harness::new(text_plan(), API_BASE, vec![answer(200, &[], b"{}")])
        .with_supplied(Mode::Asynchronous, supplied.clone());
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert!(supplied.wire.seen().is_empty());
    assert_eq!(harness.seen().len(), 1);
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

// a verb the HTTP client has no method for fails naming it, where LiteLLM raises AttributeError
#[tokio::test]
async fn two_step_upload_invalid_method_names_the_method() {
    let mut config = manus_config();
    config.upload_request.method = String::from("FROB");
    let mut harness = Harness::new(
        two_step_plan(config),
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
    match error {
        UploadError::Provider(provider) => {
            assert_eq!(provider.status_code, 500);
            assert!(provider.status_code_is_synthesized);
            assert_eq!(
                provider.message,
                "the upload method FROB is not an HTTP method"
            );
        }
        other => panic!("expected a provider error, found {other}"),
    }
    // the initial request went out and the failure stopped the second one
    assert_eq!(harness.seen().len(), 1);
}
