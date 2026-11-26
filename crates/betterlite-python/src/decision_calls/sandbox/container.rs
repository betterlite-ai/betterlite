use std::collections::BTreeMap;

use betterlite_models::decisions::{Extra, HiddenParams, Json};
use betterlite_models::sandbox::{ContainerHandle, HiddenParams as SandboxHiddenParams};
use betterlite_provider::opensandbox::lifecycle::SandboxHandle;
use betterlite_provider::sandbox_dispatch::request::SandboxContainer;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::convert::{Answer, Arguments, ConvertError, encode};
use super::{secret_json, text_json};

// The created container as LiteLLM's ContainerHandle dumps, with the tokens LiteLLM keeps private.
pub(crate) fn container_answer(container: &SandboxContainer) -> Result<String, ConvertError> {
    match container {
        SandboxContainer::E2b(handle) => {
            let hidden = e2b_hidden(handle);
            encode(&Answer {
                response: handle,
                hidden_params: &hidden,
            })
        }
        SandboxContainer::OpenSandbox(handle) => {
            let hidden = opensandbox_hidden(handle);
            encode(&Answer {
                response: &handle.container,
                hidden_params: &hidden,
            })
        }
    }
}

// A run or a delete takes the handle acreate_sandbox returned, or a bare sandbox id.
pub(crate) fn container_of(arguments: &Arguments) -> Result<SandboxContainer, ConvertError> {
    let Some(container) = &arguments.container else {
        return Err(ConvertError::Container);
    };
    match container {
        Json::Object(fields) => handle_container(fields, arguments),
        Json::Text(id) => Ok(bare_container(id, arguments)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(ConvertError::Container)
        }
    }
}

fn bare_container(id: &str, arguments: &Arguments) -> SandboxContainer {
    SandboxContainer::E2b(ContainerHandle {
        id: id.to_owned(),
        provider: arguments.provider.clone(),
        domain: None,
        hidden_params: SandboxHiddenParams::default(),
    })
}

fn handle_container(
    fields: &Extra,
    arguments: &Arguments,
) -> Result<SandboxContainer, ConvertError> {
    let Some(id) = text(fields.get("id")) else {
        return Err(ConvertError::Container);
    };
    let provider = text(fields.get("provider")).unwrap_or_else(|| arguments.provider.clone());
    let domain = text(fields.get("domain"));
    let hidden = match fields.get("hidden_params") {
        Some(Json::Object(hidden)) => hidden.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Text(_)) => {
            Extra::new()
        }
    };
    Ok(match text(hidden.get("execd_endpoint")) {
        Some(endpoint) => SandboxContainer::OpenSandbox(SandboxHandle {
            container: ContainerHandle {
                id,
                provider,
                domain,
                hidden_params: container_hidden(&hidden),
            },
            execd_endpoint: endpoint,
            execd_headers: headers(hidden.get("execd_headers")),
            use_server_proxy: proxy_flag(&hidden, arguments.use_server_proxy.unwrap_or_default()),
        }),
        None => SandboxContainer::E2b(ContainerHandle {
            id,
            provider,
            domain,
            hidden_params: e2b_params(&hidden),
        }),
    })
}

// LiteLLM reads the handle's own flag first and falls back to the run call's argument
fn proxy_flag(hidden: &Extra, fallback: bool) -> bool {
    match hidden.get("use_server_proxy") {
        Some(Json::Bool(value)) => *value,
        None
        | Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            fallback
        }
    }
}

fn e2b_params(hidden: &Extra) -> SandboxHiddenParams {
    SandboxHiddenParams {
        envd_access_token: secret(hidden.get("envd_access_token")),
        traffic_access_token: secret(hidden.get("traffic_access_token")),
        api_key: secret(hidden.get("api_key")),
        api_base: text(hidden.get("api_base")),
    }
}

fn container_hidden(hidden: &Extra) -> SandboxHiddenParams {
    SandboxHiddenParams {
        envd_access_token: None,
        traffic_access_token: None,
        api_key: secret(hidden.get("api_key")),
        api_base: text(hidden.get("api_base")),
    }
}

fn e2b_hidden(handle: &ContainerHandle) -> HiddenParams {
    let params = &handle.hidden_params;
    hidden_map(&[
        (
            "envd_access_token",
            secret_json(params.envd_access_token.as_ref()),
        ),
        (
            "traffic_access_token",
            secret_json(params.traffic_access_token.as_ref()),
        ),
        ("api_key", secret_json(params.api_key.as_ref())),
        ("api_base", text_json(params.api_base.as_deref())),
    ])
}

fn opensandbox_hidden(handle: &SandboxHandle) -> HiddenParams {
    let params = &handle.container.hidden_params;
    let api_base = params
        .api_base
        .as_deref()
        .or(handle.container.domain.as_deref());
    let mut hidden = hidden_map(&[
        ("api_base", text_json(api_base)),
        ("api_key", secret_json(params.api_key.as_ref())),
    ]);
    hidden.insert(
        String::from("execd_endpoint"),
        Json::Text(handle.execd_endpoint.clone()),
    );
    hidden.insert(
        String::from("execd_headers"),
        Json::Object(
            handle
                .execd_headers
                .iter()
                .map(|(name, value)| (name.clone(), Json::Text(value.expose_secret().to_owned())))
                .collect(),
        ),
    );
    hidden.insert(
        String::from("use_server_proxy"),
        Json::Bool(handle.use_server_proxy),
    );
    hidden
}

fn hidden_map(entries: &[(&str, Json)]) -> HiddenParams {
    let mut hidden = HiddenParams::default();
    for (name, value) in entries {
        hidden.insert(String::from(*name), value.clone());
    }
    hidden
}

fn headers(value: Option<&Json>) -> BTreeMap<String, SecretString> {
    let Some(Json::Object(fields)) = value else {
        return BTreeMap::new();
    };
    let mut headers = BTreeMap::new();
    for (name, header_value) in fields {
        if let Some(found) = text(Some(header_value)) {
            headers.insert(name.clone(), SecretString::from(found.as_str()));
        }
    }
    headers
}

fn secret(value: Option<&Json>) -> Option<SecretString> {
    let found = text(value)?;
    if found.is_empty() {
        return None;
    }
    Some(SecretString::from(found.as_str()))
}

fn text(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}
