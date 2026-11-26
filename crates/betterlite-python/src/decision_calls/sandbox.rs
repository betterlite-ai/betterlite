use betterlite_models::decisions::{Extra, Json};
use betterlite_provider::sandbox_dispatch::registry::SandboxTool;
use betterlite_provider::sandbox_dispatch::request::{
    CreateSandbox, DeleteSandbox, SandboxCall, SandboxContainer, SandboxContainerRef,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::convert::{Arguments, ConvertError, encode, requested_timeout};

pub(crate) mod container;
pub(crate) mod run;
mod tests;

// The tool registry hands back what LiteLLM's resolve_sandbox_tool answers with, or None.
pub(crate) fn tool_answer(tool: Option<&SandboxTool>) -> Result<String, ConvertError> {
    let answer = tool.map_or_else(
        || Json::Null,
        |found| {
            let mut fields = Extra::new();
            fields.insert(
                String::from("sandbox_provider"),
                Json::Text(found.provider.clone()),
            );
            fields.insert(String::from("api_key"), secret_json(found.api_key.as_ref()));
            fields.insert(
                String::from("api_base"),
                text_json(found.api_base.as_deref()),
            );
            Json::Object(fields)
        },
    );
    encode(&answer)
}

macro_rules! sandbox_request {
    ($request:ident, $arguments:expr, { $($own:ident: $value:expr),* $(,)? }) => {
        $request {
            $($own: $value,)*
            call: sandbox_call($arguments),
            api_base: $arguments.api_base.as_deref(),
            env_vars: $arguments.env_vars.as_ref(),
            use_server_proxy: $arguments.use_server_proxy.unwrap_or_default(),
            ready_timeout: requested_timeout($arguments.ready_timeout)?,
            poll_interval: requested_timeout($arguments.poll_interval)?,
        }
    };
}

pub(crate) fn create_request<'call>(
    arguments: &'call Arguments,
    policy: Option<&'call RawValue>,
) -> Result<CreateSandbox<'call>, ConvertError> {
    Ok(sandbox_request!(CreateSandbox, arguments, {
        template: arguments.template.as_deref(),
        timeout: arguments.create_timeout,
        allow_internet_access: arguments.allow_internet_access,
        metadata: arguments.metadata.as_ref(),
        resource_limits: arguments.resource_limits.as_ref(),
        resource_requests: arguments.resource_requests.as_ref(),
        entrypoint: arguments.entrypoint.as_deref(),
        network_policy: policy,
        secure_access: arguments.secure_access.unwrap_or_default(),
    }))
}

pub(crate) fn delete_request<'call>(
    arguments: &'call Arguments,
    container: &'call SandboxContainer,
) -> DeleteSandbox<'call> {
    let container_ref = SandboxContainerRef::Container(container);
    DeleteSandbox {
        api_base: arguments.api_base.as_deref(),
        call: sandbox_call(arguments),
        container: container_ref,
    }
}

pub(crate) fn network_policy(value: Option<&Json>) -> Result<Option<Box<RawValue>>, ConvertError> {
    let Some(policy) = value else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_string(policy).map_err(|source| ConvertError::Policy { source })?;
    RawValue::from_string(encoded)
        .map(Some)
        .map_err(|source| ConvertError::Policy { source })
}

fn sandbox_call(arguments: &Arguments) -> SandboxCall<'_> {
    SandboxCall {
        provider: &arguments.provider,
        api_key: arguments.api_key.as_ref(),
        kwargs: &arguments.params,
        logging: None,
    }
}

fn secret_json(value: Option<&SecretString>) -> Json {
    value.map_or(Json::Null, |secret| {
        Json::Text(secret.expose_secret().to_owned())
    })
}

fn text_json(value: Option<&str>) -> Json {
    value.map_or_else(|| Json::Null, |text| Json::Text(text.to_owned()))
}
