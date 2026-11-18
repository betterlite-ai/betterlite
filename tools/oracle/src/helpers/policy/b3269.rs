use std::collections::BTreeMap;

use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_destination::parameters::tags_from_request_body;
use betterlite_core::api::response_id_codecs::response::get_model_id_from_response_id;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::registry::{PolicyRegistry, PolicySource};
use betterlite_policy::response_retrieval::{
    PolicyRouterView, RetrievalAuth, RetrievalData, RetrievalDependencies, RetrievalRequest,
    attach_retrieval_pipelines,
};
use betterlite_policy::values::{Policy, PolicyAttachment};
use serde::Deserialize;
use serde_json::{Value, json};

mod logging;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = registry_for(input.policies.as_ref());
        let mut attachments = AttachmentRegistry::default();
        attachments.load_attachments(input.attachments.clone());
        let mut data = input.data.clone();
        let tags = tags_from_request_body(&request_data(&data)?);
        let decoder: &dyn Fn(Option<&str>) -> Option<String> = &get_model_id_from_response_id;
        let deps = RetrievalDependencies {
            registry: &registry,
            attachments: &attachments,
            router: input
                .router
                .as_ref()
                .map(|view| -> &dyn PolicyRouterView { view }),
            model_id_from_response_id: decoder,
        };
        let request = RetrievalRequest {
            auth: RetrievalAuth {
                team_alias: input.auth.team_alias.as_deref(),
                key_alias: input.auth.key_alias.as_deref(),
            },
            tags,
        };
        let ((), logs) =
            logging::captured(|| attach_retrieval_pipelines(&mut data, &request, &deps));
        Ok(json!({
            "data": data,
            "logs": logs,
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    data: RetrievalData,
    auth: Auth,
    router: Option<RouterInput>,
    policies: Option<BTreeMap<String, Policy>>,
    attachments: Vec<PolicyAttachment>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Auth {
    team_alias: Option<String>,
    key_alias: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RouterInput {
    deployments: BTreeMap<String, String>,
    aliases: Vec<(String, String)>,
}

impl PolicyRouterView for RouterInput {
    fn deployment_model_name(&self, model_id: &str) -> Option<String> {
        self.deployments.get(model_id).cloned()
    }

    fn model_group_aliases(&self) -> Vec<(String, String)> {
        self.aliases.clone()
    }
}

// LiteLLM's registry is initialized by any load, even of an empty config, and never by a null one
fn registry_for(policies: Option<&BTreeMap<String, Policy>>) -> PolicyRegistry {
    let mut registry = PolicyRegistry::default();
    let Some(configured) = policies else {
        return registry;
    };
    for (policy_name, policy) in configured {
        registry.add_policy(policy_name, policy.clone(), PolicySource::Config);
    }
    registry.initialized = true;
    registry
}

// the proxy extracts the retrieval tags from the request body it already holds
fn request_data(data: &RetrievalData) -> Result<RequestData, Box<HelperError>> {
    let encoded: Value = serde_json::to_value(data)?;
    let request_data = serde_json::from_value(encoded)?;
    Ok(request_data)
}
