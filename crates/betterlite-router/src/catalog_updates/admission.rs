use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;

const ENVIRONMENT_PREFIX: &str = "os.environ/";

// _add_deployment's edits: a router-level rpm or tpm fills a missing one, and a data source os.environ/ reference becomes its value.
pub fn normalize(spec: &mut DeploymentSpec, environment: &dyn EnvironmentLookup) {
    for limit in ["rpm", "tpm"] {
        let unset = matches!(spec.litellm_params.get(limit), None | Some(Json::Null));
        let router_level = spec
            .extra
            .get(limit)
            .filter(|value| !matches!(value, Json::Null));
        if let (true, Some(value)) = (unset, router_level) {
            spec.litellm_params.insert(limit.to_owned(), value.clone());
        }
    }
    let Some(Json::List(sources)) = spec.litellm_params.get_mut("dataSources") else {
        return;
    };
    for source in sources {
        let Json::Object(fields) = source else {
            continue;
        };
        let Some(Json::Object(parameters)) = fields.get_mut("parameters") else {
            continue;
        };
        for key in ["endpoint", "key"] {
            if let Some(Json::Text(text)) = parameters.get_mut(key)
                && text.starts_with(ENVIRONMENT_PREFIX)
            {
                *text = environment
                    .lookup(&text.replace(ENVIRONMENT_PREFIX, ""))
                    .unwrap_or_default();
            }
        }
    }
}
