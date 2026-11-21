use std::fmt::Display;

use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_setup_options::CallbackVarsDecryption;
use betterlite_core::api::request_setup_options::RequestSetupOptions;
use betterlite_core::api::request_setup_options::callbacks::CallbackSettings;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_a::environment::EnvironmentLookup;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

pub const TRUSTED_CALLBACK_VARS_FIELD: &str = "litellm_trusted_callback_vars";
const ENVIRONMENT_REFERENCE: &str = "os.environ/";
const METADATA_KEY: &str = "metadata";
const MODEL_INFO_KEY: &str = "model_info";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamCallbackWiring {
    pub success_callbacks: Option<Vec<String>>,
    pub failure_callbacks: Option<Vec<String>>,
    pub logging_kwargs: Option<Metadata>,
}

// _resolve_team_callback_wiring: the key's and team's logging callbacks, or none when resolving fails
#[must_use]
pub fn resolve_team_callback_wiring<Ids, Decryption, Environment>(
    options: &RequestSetupOptions<Ids, Decryption, Environment>,
    user_api_key_dict: &PrincipalFacts,
    route_description: &str,
) -> TeamCallbackWiring
where
    Decryption: CallbackVarsDecryption,
    Environment: EnvironmentLookup,
{
    let mut diagnostics = Vec::new();
    let settings = match options.dynamic_logging_settings(user_api_key_dict, &mut diagnostics) {
        Ok(settings) => settings,
        Err(error) => return fail_open(route_description, &error),
    };
    let Some(settings) = settings else {
        return TeamCallbackWiring::default();
    };
    if has_environment_reference(&settings.callback_vars) {
        return fail_open(
            route_description,
            "a callback variable is an os.environ/ reference",
        );
    }
    TeamCallbackWiring {
        success_callbacks: Some(settings.success_callback.clone()),
        failure_callbacks: Some(settings.failure_callback.clone()),
        logging_kwargs: callback_logging_kwargs(&settings),
    }
}

// a broken logging config must never fail the customer's upstream call
fn fail_open(route_description: &str, reason: &dyn Display) -> TeamCallbackWiring {
    tracing::error!(
        "{route_description}: failed to resolve team logging callbacks, continuing without them: {reason}"
    );
    TeamCallbackWiring::default()
}

// the deprecated callback_settings branch skips AddTeamCallback validation, so Logging.__init__ would reject these
fn has_environment_reference(callback_vars: &IndexMap<String, SecretString>) -> bool {
    for value in callback_vars.values() {
        if value.expose_secret().contains(ENVIRONMENT_REFERENCE) {
            return true;
        }
    }
    false
}

fn callback_logging_kwargs(settings: &CallbackSettings) -> Option<Metadata> {
    if settings.callback_vars.is_empty() {
        return None;
    }
    let mut trusted = Metadata::new();
    for (name, value) in &settings.callback_vars {
        trusted.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let mut kwargs = trusted.clone();
    kwargs.insert(
        TRUSTED_CALLBACK_VARS_FIELD.to_owned(),
        Json::Object(trusted),
    );
    kwargs.insert(METADATA_KEY.to_owned(), Json::Object(Metadata::new()));
    kwargs.insert(MODEL_INFO_KEY.to_owned(), Json::Object(Metadata::new()));
    Some(kwargs)
}
