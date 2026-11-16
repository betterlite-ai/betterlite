use betterlite_http::error::HttpError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::data_cloud_dispatch::context::{EnvironmentSource, oci_explicit_params};
use betterlite_provider::data_cloud_dispatch::error::DispatchError;
use betterlite_provider::data_cloud_dispatch::oci_stream::{OciStreamRequest, open_stream};
use betterlite_provider::data_cloud_dispatch::transport::{PostTransport, StreamHead};
use betterlite_provider::oci_chat::config::{
    get_complete_url as oci_url, validate_environment as oci_headers,
};
use betterlite_provider::oci_common::credentials::resolve_oci_credentials;
use betterlite_provider::oci_common::environment::OciEnvironment;
use futures_util::stream::{self, StreamExt as _};
use http::HeaderMap;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::chat::chat_messages;
use super::fixtures::{ORACLE_UPSTREAM, Scripted, environment, secrets};
use super::input::Case;
use super::wire::{error_or_null, request_view};

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let secrets = secrets();
    let source = EnvironmentSource {
        secrets: &secrets,
        environment: &env,
    };
    let (url, headers) = stream_target(case, source)?;
    let transport = Scripted::with_stream(stream_head(case));
    let data = Json::Object(Object::new());
    let outcome = runtime.block_on(drain(
        OciStreamRequest {
            api_base: &url,
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    ));
    let request = match transport.first() {
        Some(sent) => request_view(&sent.url, &sent.headers, case.provider.kept_headers(), None),
        None => Value::Null,
    };
    Ok(json!({"request": request, "error": error_or_null(&outcome)}))
}

// the OCI chat config decides the url and the headers the stream wrapper sends
fn stream_target(
    case: &Case,
    source: EnvironmentSource<'_>,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let optional_params = case.optional_params()?;
    let explicit = oci_explicit_params(&optional_params).or_value_error()?;
    let oci_environment = source.oci().or_value_error()?;
    let credentials = resolve_oci_credentials(&explicit, &oci_environment);
    let mut headers = HeaderMap::new();
    let messages = chat_messages(&case.messages)?;
    oci_headers(
        &mut headers,
        &messages,
        &credentials,
        &OciEnvironment::default(),
    )
    .or_value_error()?;
    let api_base = stream_api_base(case)?;
    let url = oci_url(&credentials, Some(&api_base)).or_value_error()?;
    Ok((url, headers))
}

fn stream_api_base(case: &Case) -> Result<String, Box<HelperError>> {
    let Some(api_base) = case.api_base.as_deref() else {
        return Err(super::input_failure(String::from(
            "the oci_stream operation needs an api_base",
        )));
    };
    Ok(api_base.replace("{{upstream}}", ORACLE_UPSTREAM))
}

async fn drain(
    request: OciStreamRequest<'_>,
    transport: &dyn PostTransport,
) -> Result<(), DispatchError> {
    let mut stream = open_stream(request, transport).await?;
    while stream.next().await?.is_some() {}
    Ok(())
}

fn stream_head(case: &Case) -> StreamHead {
    let body = case.answer_body();
    let status = case.answer_status();
    // a checked division keeps the arithmetic lint quiet; the middle index never exceeds the body length
    let middle = body.len().checked_div(2).unwrap_or_default();
    let (first, second) = body.as_bytes().split_at(middle);
    let chunks: [Result<Vec<u8>, HttpError>; 2] = [Ok(first.to_vec()), Ok(second.to_vec())];
    StreamHead {
        status,
        body: if status == 200 {
            Vec::new()
        } else {
            body.clone().into_bytes()
        },
        chunks: (status == 200).then_some(stream::iter(chunks).boxed()),
    }
}
