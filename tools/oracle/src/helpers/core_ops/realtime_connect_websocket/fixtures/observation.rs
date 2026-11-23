use std::sync::PoisonError;

use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_core::api::realtime_connect::{
    Branch, ConnectionQuery, RealtimeConnectParams, RealtimeConnection,
};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::fixture::ScriptedTokens;

pub(crate) fn observation<Config>(
    connection: RealtimeConnection<Config>,
    tokens: &ScriptedTokens,
) -> Value {
    let RealtimeConnection { args, branch } = connection;
    let (tag, details) = branch_observation(branch, tokens);
    json!({
        "branch": tag,
        "model": args.model,
        "api_base": args.api_base,
        "api_key": args.api_key.as_ref().map(|key| key.expose_secret()),
        "timeout": args.timeout,
        "query": query_pairs(&args.query_params),
        "metadata": args.litellm_metadata.as_ref().map_or(Json::Null, |metadata| Json::Object(metadata.clone())),
        "principal": args.user_api_key_dict,
        "headers": header_pairs(args.headers.as_ref()),
        "details": details,
    })
}

fn branch_observation<Config>(
    branch: Branch<Config>,
    tokens: &ScriptedTokens,
) -> (&'static str, Value) {
    match branch {
        Branch::ProviderConfig { tag, .. } => (provider_tag(tag), json!({})),
        Branch::Azure {
            api_version,
            protocol,
            azure_ad_token,
        } => (
            "azure",
            json!({
                "api_version": api_version,
                "protocol": protocol,
                "azure_ad_token": azure_ad_token.as_ref().map(|token| token.expose_secret()),
            }),
        ),
        Branch::OpenAi => ("openai", json!({})),
        Branch::Xai => ("xai", json!({})),
        Branch::Bedrock(params) => ("bedrock", bedrock_details(&params)),
        Branch::Vertex {
            access_token,
            project,
            location,
            ..
        } => (
            "vertex",
            json!({
                "access_token": access_token.expose_secret(),
                "project": project,
                "location": location,
                "token_calls": token_calls(tokens),
            }),
        ),
    }
}

fn provider_tag(tag: RealtimeConfigTag) -> &'static str {
    match tag {
        RealtimeConfigTag::Gemini => "gemini",
        RealtimeConfigTag::Meta => "meta",
    }
}

fn query_pairs(query: &ConnectionQuery) -> Value {
    let mut pairs: Vec<(&str, Option<&str>)> = match query {
        ConnectionQuery::Omitted => Vec::new(),
        ConnectionQuery::Transport(query) => query
            .iter()
            .flatten()
            .map(|(name, value)| (name.as_str(), Some(value.as_str())))
            .collect(),
        ConnectionQuery::Connector(query) => query
            .iter()
            .flatten()
            .map(|(name, value)| (name.as_str(), value.as_deref()))
            .collect(),
    };
    pairs.sort_unstable();
    json!(pairs)
}

fn header_pairs(headers: Option<&HeaderMap>) -> Value {
    let pairs: Option<Vec<(&String, &Json)>> = headers.map(|values| values.iter().collect());
    json!(pairs)
}

fn bedrock_details(params: &RealtimeConnectParams) -> Value {
    json!({
        "aws_region_name": params.aws_region_name,
        "aws_access_key_id": params.aws_access_key_id.as_ref().map(|value| value.expose_secret()),
        "aws_secret_access_key": params.aws_secret_access_key.as_ref().map(|value| value.expose_secret()),
        "aws_session_token": params.aws_session_token.as_ref().map(|value| value.expose_secret()),
        "aws_role_name": params.aws_role_name,
        "aws_session_name": params.aws_session_name,
        "aws_profile_name": params.aws_profile_name,
        "aws_web_identity_token": params.aws_web_identity_token.as_ref().map(|value| value.expose_secret()),
        "aws_sts_endpoint": params.aws_sts_endpoint,
        "aws_bedrock_runtime_endpoint": params.aws_bedrock_runtime_endpoint,
        "aws_external_id": params.aws_external_id,
        "aws_session_tags": params.aws_session_tags,
    })
}

fn token_calls(tokens: &ScriptedTokens) -> Value {
    let calls = tokens
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!(calls)
}
