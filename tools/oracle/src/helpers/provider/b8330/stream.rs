use betterlite_http::error::HttpError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::data_cloud_dispatch::context::{oci_environment, oci_explicit_params};
use betterlite_provider::data_cloud_dispatch::error::DispatchError;
use betterlite_provider::data_cloud_dispatch::oci_stream::{OciStreamRequest, open_stream};
use betterlite_provider::data_cloud_dispatch::transport::StreamHead;
use betterlite_provider::oci_chat::config::{
    get_complete_url as oci_url, validate_environment as oci_headers,
};
use betterlite_provider::oci_chat::stream::StreamItem;
use betterlite_provider::oci_common::credentials::resolve_oci_credentials;
use betterlite_provider::oci_common::environment::OciEnvironment;
use futures_util::stream::{self, StreamExt as _};
use http::HeaderMap;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::chat::chat_messages;
use super::fixture::{Scripted, environment, secrets, substitute};
use super::input::Case;
use super::wire::{error_view, kept_headers};

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let secrets = secrets();
    let optional_params = case.optional_params()?;
    let explicit = oci_explicit_params(&optional_params).or_value_error()?;
    let oci_env = oci_environment(&secrets, &env).or_value_error()?;
    let credentials = resolve_oci_credentials(&explicit, &oci_env);
    let mut headers = HeaderMap::new();
    let messages = chat_messages(&case.messages)?;
    oci_headers(
        &mut headers,
        &messages,
        &credentials,
        &OciEnvironment::default(),
    )
    .or_value_error()?;
    let api_base = match case.api_base.as_deref() {
        Some(value) => substitute(value),
        None => {
            return Err(super::input_failure(String::from(
                "the oci_stream operation needs an api_base",
            )));
        }
    };
    let url = oci_url(&credentials, Some(&api_base)).or_value_error()?;
    let transport = Scripted::with_stream(stream_head(case));
    let data = Json::Object(Object::new());
    let outcome = runtime.block_on(async {
        let mut decoded = String::new();
        let mut stream = open_stream(
            OciStreamRequest {
                api_base: &url,
                headers,
                data: &data,
                signed_body: None,
            },
            &transport,
        )
        .await?;
        while let Some(item) = stream.next().await? {
            if let StreamItem::Chunk(chunk) = item
                && let Some(content) = chunk.choice.delta.content
            {
                decoded.push_str(&content);
            }
        }
        Ok::<String, DispatchError>(decoded)
    });
    let request = match transport.first() {
        Some(sent) => json!({
            "url": sent.url,
            "headers": kept_headers(&sent.headers, case.provider.kept_headers()),
        }),
        None => Value::Null,
    };
    let (reply, error) = match &outcome {
        Ok(content) => (json!({"content": content}), Value::Null),
        Err(failure) => (Value::Null, error_view(failure)),
    };
    Ok(json!({"request": request, "reply": reply, "error": error}))
}

fn stream_head(case: &Case) -> StreamHead {
    let body = case.answer_body();
    let status = case.answer_status();
    let middle = body.len() / 2;
    let chunks: Vec<Result<Vec<u8>, HttpError>> = vec![
        Ok(body.as_bytes()[..middle].to_vec()),
        Ok(body.as_bytes()[middle..].to_vec()),
    ];
    StreamHead {
        status,
        body: if status == 200 {
            Vec::new()
        } else {
            body.clone().into_bytes()
        },
        chunks: (status == 200).then(|| stream::iter(chunks).boxed()),
    }
}
