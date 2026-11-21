pub mod body;
pub mod client;
pub mod error;
pub mod operation;
pub mod outcome;
pub mod request;

mod handlers;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_config::sections::assistant_settings::Settings;
use betterlite_config::sections::azure_endpoints;
use betterlite_config::sections::resource_sdk_settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::stream::{RunStream, RunStreamTransport};
use betterlite_redact::secret::Redactor;

use self::error::AssistantDispatchError;
use self::outcome::AssistantOutcome;
use self::request::AssistantRequest;

// router.py#L7188-L7190: the exact text the factory raises when no provider is configured or passed
const REQUIRED_PROVIDER: &str = "'custom_llm_provider' must be set. Either via:\n `Router(assistants_config={'custom_llm_provider': ..})` \nor\n `router.arun_thread(custom_llm_provider=..)`";

// The callee Router.initialize_assistants_endpoint installs for the eight assistant wrappers.
pub struct AssistantDispatcher<'deps, 'resolution, T, S> {
    // assistant_settings: Router(assistants_config=..)
    pub settings: &'deps Settings,
    pub resources: &'deps resource_sdk_settings::Settings,
    pub azure: &'deps azure_endpoints::Settings,
    pub environment: &'deps BTreeMap<String, String>,
    pub sources: S,
    pub resolution: &'resolution ResolutionContext<'deps>,
    pub redactor: &'deps Redactor,
    pub transport: Arc<T>,
    pub limits: ApiLimits,
}

struct Merged {
    provider: String,
    kwargs: BTreeMap<String, Json>,
}

// the wrapper's own named arguments: a configured copy of either collides with the call (router.py#L7191)
fn check_arguments(
    function: &str,
    kwargs: &BTreeMap<String, Json>,
) -> Result<(), AssistantDispatchError> {
    for name in ["custom_llm_provider", "client"] {
        if kwargs.contains_key(name) {
            return Err(AssistantDispatchError::duplicate_argument(function, name));
        }
    }
    Ok(())
}

// the caller's explicit provider, or None for the two values Python treats as unset
fn caller_provider(value: Option<&Json>) -> Result<Option<String>, AssistantDispatchError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(provider)) => Ok(Some(provider.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AssistantDispatchError::invalid("custom_llm_provider"))
        }
    }
}

impl<T, S> AssistantDispatcher<'_, '_, T, S>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    // _pass_through_assistants_endpoint_factory: bind the caller's provider and client, overlay the configured parameters when the provider is null, then call the installed function.
    pub async fn dispatch(
        &self,
        request: &AssistantRequest<'_>,
    ) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError> {
        let (function, path) = request.operation.installed();
        if let Some(name) = path
            && request.data.contains_key(name)
        {
            // the route passes this id as a keyword next to **data, so a body key of that name collides
            return Err(AssistantDispatchError::multiple_values(name));
        }
        let caller = caller_provider(request.data.get("custom_llm_provider"))?;
        let mut kwargs = request.data.clone();
        kwargs.remove("custom_llm_provider");
        kwargs.remove("client");
        if let (Some(name), Some(value)) = (path, request.path_value()) {
            // the route's keyword is in kwargs before the factory runs, so the overlay can replace it
            kwargs.insert(name.to_owned(), Json::Text(value.to_owned()));
        }
        let provider = match caller {
            Some(provider) => provider,
            None => self.overlay(&mut kwargs)?,
        };
        check_arguments(function, &kwargs)?;
        handlers::run(self, request, Merged { provider, kwargs }).await
    }

    fn overlay(
        &self,
        kwargs: &mut BTreeMap<String, Json>,
    ) -> Result<String, AssistantDispatchError> {
        let Some(config) = self.settings.config.as_ref() else {
            return Err(AssistantDispatchError::Exception {
                message: String::from(REQUIRED_PROVIDER),
            });
        };
        // AssistantsTypedDict is a runtime dict: the two keys are plain dictionary reads
        let provider =
            config
                .custom_llm_provider
                .clone()
                .ok_or(AssistantDispatchError::KeyError {
                    key: "custom_llm_provider",
                })?;
        let params = config
            .litellm_params
            .as_ref()
            .ok_or(AssistantDispatchError::KeyError {
                key: "litellm_params",
            })?;
        kwargs.extend(params.clone());
        Ok(provider)
    }
}
