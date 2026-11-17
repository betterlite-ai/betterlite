use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::containers::container_file_http::proxied::reply_headers;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::gitlab_client::payload::decode_text;
use secrecy::ExposeSecret;

use crate::data_cloud_dispatch::call::{EmbeddingCall, EmbeddingReply};
use crate::data_cloud_dispatch::embed::EmbeddingContext;
use crate::data_cloud_dispatch::error::DispatchError;
use crate::snowflake::config::{
    Endpoint as SnowflakeEndpoint, get_complete_url, validate_environment,
};
use crate::snowflake::embedding::{
    get_error_class, transform_embedding_request, transform_embedding_response,
};

use super::send_json;

// B5844's get_complete_url and transform_embedding_request decide the url and the body
fn request(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<(String, Object), DispatchError> {
    let account_id = context.env.text("SNOWFLAKE_ACCOUNT_ID")?;
    let mut optional_params: Object = call.optional_params.clone();
    let url = get_complete_url(
        call.api_base.as_deref(),
        &SnowflakeEndpoint::Embedding,
        &mut optional_params,
        account_id.as_deref(),
    )?;
    Ok((
        url,
        transform_embedding_request(&call.input, &call.model, &optional_params),
    ))
}

// B5844's transform_embedding_response turns a 2xx body into the reply, and its error class takes the rest
fn reply(model: &str, response: &UpstreamResponse) -> Result<EmbeddingReply, DispatchError> {
    if !(200..300).contains(&response.status) {
        return Err(DispatchError::SnowflakeException(get_error_class(
            &decode_text(&response.body),
            response.status,
            reply_headers(&response.headers),
        )));
    }
    let reply = transform_embedding_response(model, &decode_text(&response.body))?;
    Ok(EmbeddingReply::Snowflake(Box::new(reply)))
}

// The snowflake arm takes the key, the url and the body, sends them and maps the answer
pub(super) async fn complete(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    let api_key = match call
        .api_key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())
    {
        Some(key) => Some(key.clone()),
        None => context.env.secret("SNOWFLAKE_JWT")?,
    };
    // the snowflake embedding arm starts from an empty map: the caller's extra headers never reach this provider
    let mut headers = Headers::new();
    validate_environment(
        &mut headers,
        &call.model,
        api_key.as_ref().map(ExposeSecret::expose_secret),
    )?;
    let (url, body) = request(call, context)?;
    let response = send_json(context, &url, &headers, &body, call.timeout).await?;
    reply(&call.model, &response)
}
