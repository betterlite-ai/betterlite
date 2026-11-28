#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_redact::secret::Redactor;
use tokio::runtime::Builder;

use super::cases::{Answer, STABILITY_MODEL, answered, auth_params, call, image_list, run, secret};
use super::fixture::{Factory, FixtureClient, Reply};
use crate::image_edits::execute::dispatch_request;
use crate::image_edits::plan::PreparedRequest;
use crate::signing::request::AwsSignedRequest;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

pub(super) fn edit_signed_request(redactor: &Redactor) {
    signed_signature(redactor);
    signed_extra_headers(redactor);
    signed_bearer_token(redactor);
    signed_authorization_precedence(redactor);
    signed_configured_endpoint(redactor);
    signed_api_base(redactor);
    signed_record_mismatch();
}

fn signed_signature(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    let mut signed_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    signed_call.images = &supplied;
    let signed = run(
        signed_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        signed.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/stability.stable-image-inpaint-v1:0/invoke"
    );
    assert_eq!(
        signed.text(),
        r#"{"output_format": "png", "prompt": "edit it", "image": "aW1n"}"#
    );
    let headers = signed.headers();
    assert_eq!(headers["Content-Type"], "application/json");
    assert_eq!(headers["Content-Length"], signed.text().len().to_string());
    assert!(headers["Authorization"].starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20231114/us-east-1/bedrock/aws4_request"
    ));
    assert!(headers["Authorization"].contains("SignedHeaders=content-type;host;x-amz-date"));
    assert_eq!(headers["X-Amz-Date"], "20231114T221320Z");
    signed_pre_call_log(redactor);
}

fn signed_pre_call_log(redactor: &Redactor) {
    // a failed status skips post_call, so the details still hold what pre_call recorded
    let supplied = image_list(&["aW1n"]);
    let mut failed_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    failed_call.images = &supplied;
    let failed = run(
        failed_call,
        "{}",
        Answer {
            reply: Reply::status(500, &[], "{}"),
            supplied: Some(ClientKind::Sync),
        },
        redactor,
    );
    assert_eq!(
        failed.state.model_call_details.get("input"),
        Some(&Json::Text("edit it".to_owned()))
    );
    let arguments = match failed.state.model_call_details.get("additional_args") {
        Some(Json::Object(arguments)) => arguments,
        other => panic!("pre_call records its arguments, got {other:?}"),
    };
    assert_eq!(
        arguments.get("api_base"),
        Some(&Json::Text(failed.posted().url.clone()))
    );
    assert!(matches!(
        arguments.get("headers"),
        Some(Json::Object(logged)) if logged.contains_key("Authorization")
    ));
    assert_eq!(
        arguments
            .get("complete_input_dict")
            .map(|value| serde_json::to_value(value).unwrap()),
        Some(serde_json::to_value(failed.body()).unwrap())
    );
}

fn signed_extra_headers(redactor: &Redactor) {
    // an extra header of an exact name replaces the built-in one; a new one rides along
    let supplied = image_list(&["aW1n"]);
    let extra = [
        ("X-Custom".to_owned(), "v".to_owned()),
        (
            "Content-Type".to_owned(),
            "application/json; charset=utf-8".to_owned(),
        ),
    ];
    let mut customized_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    customized_call.images = &supplied;
    customized_call.extra_headers = Some(&extra);
    let customized = run(
        customized_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    let custom_headers = customized.headers();
    assert_eq!(
        custom_headers["Content-Type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(custom_headers["X-Custom"], "v");
    assert!(!customized.body().contains_key("X-Custom"));
}

fn signed_bearer_token(redactor: &Redactor) {
    // an api_key signs as a bearer token and drops the SigV4 date
    let supplied = image_list(&["aW1n"]);
    let token = secret("br-token");
    let mut bearer_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    bearer_call.images = &supplied;
    bearer_call.api_key = Some(&token);
    let bearer = run(
        bearer_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(bearer.headers()["Authorization"], "Bearer br-token");
    assert!(!bearer.headers().contains_key("X-Amz-Date"));
    // a configured bearer token from the environment signs the same way
    let configured = secret("env-token");
    let mut environment_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    environment_call.images = &supplied;
    environment_call.environment.bearer_token = Some(&configured);
    let from_env = run(
        environment_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(from_env.headers()["Authorization"], "Bearer env-token");
    assert!(!from_env.headers().contains_key("X-Amz-Date"));
}

fn signed_authorization_precedence(redactor: &Redactor) {
    // an extra Authorization of an ordinary value replaces the SigV4 header, and the date stays
    let supplied = image_list(&["aW1n"]);
    let custom_headers = [("Authorization".to_owned(), "Custom tok".to_owned())];
    let mut custom_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    custom_call.images = &supplied;
    custom_call.extra_headers = Some(&custom_headers);
    let custom = run(
        custom_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    let replaced = custom.headers();
    assert_eq!(replaced["Authorization"], "Custom tok");
    assert_eq!(replaced["X-Amz-Date"], "20231114T221320Z");
    // an extra Authorization that already looks signed leaves the computed signature in place
    let caller_headers = [(
        "Authorization".to_owned(),
        "AWS4-HMAC-SHA256 caller".to_owned(),
    )];
    let mut caller_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    caller_call.images = &supplied;
    caller_call.extra_headers = Some(&caller_headers);
    let caller = run(
        caller_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert!(
        caller.headers()["Authorization"]
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/")
    );
}

fn signed_configured_endpoint(redactor: &Redactor) {
    // a configured endpoint wins over the default, a trailing slash doubles as Python's f-string does
    let supplied = image_list(&["aW1n"]);
    let mut endpoint_options = auth_params();
    endpoint_options.insert(
        "aws_bedrock_runtime_endpoint".to_owned(),
        Json::Text("https://runtime.example/".to_owned()),
    );
    let mut endpoint_call = call(STABILITY_MODEL, Some("edit it"), endpoint_options);
    endpoint_call.images = &supplied;
    let configured = run(
        endpoint_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        configured.posted().url,
        "https://runtime.example//model/stability.stable-image-inpaint-v1:0/invoke"
    );
}

fn signed_api_base(redactor: &Redactor) {
    // api_base is the configured endpoint the caller passes, and it wins over the default
    let supplied = image_list(&["aW1n"]);
    let mut api_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    api_call.images = &supplied;
    api_call.api_base = Some("https://api.example");
    let proxied = run(
        api_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        proxied.posted().url,
        "https://api.example/model/stability.stable-image-inpaint-v1:0/invoke"
    );
}

fn signed_record_mismatch() {
    // the transport sends the stored endpoint and prepared body, not the signed URL record
    let direct_call = call(STABILITY_MODEL, None, ImageParams::default());
    let prepared = PreparedRequest {
        endpoint_url: "https://stored.example/model/m/invoke".to_owned(),
        signed: AwsSignedRequest {
            method: "POST".to_owned(),
            url: "https://signed.example/elsewhere".to_owned(),
            headers: vec![("X-Prepared".to_owned(), "yes".to_owned())],
            body: b"{\"stored\": true}".to_vec(),
        },
        data: ImageParams::default(),
    };
    let direct_client = FixtureClient::new(Reply::ok(r#"{"images": ["aQ=="]}"#));
    let direct_factory = Factory::new(Reply::ok(r#"{"images": []}"#));
    let runtime = Builder::new_current_thread().build().unwrap();
    let reply = runtime
        .block_on(dispatch_request(
            &direct_call,
            &prepared,
            Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &direct_client,
            }),
            &direct_factory,
        ))
        .unwrap();
    assert_eq!(reply.status, 200);
    assert_stored_request(&direct_client);
}

fn assert_stored_request(client: &FixtureClient) {
    let sent_requests = client.requests();
    let sent = &sent_requests[0];
    assert_eq!(sent.url, "https://stored.example/model/m/invoke");
    assert_eq!(sent.body, b"{\"stored\": true}".to_vec());
    assert_eq!(
        sent.headers,
        vec![("X-Prepared".to_owned(), "yes".to_owned())]
    );
}
