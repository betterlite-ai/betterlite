use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_catalog::types::Candidates;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::static_eligibility::environment::model_info_is_active_for_environment;
use betterlite_router::static_eligibility::error::EligibilityError;
use betterlite_router::static_eligibility::filter::{
    EligibilityInput, EligibilityResult, EligibilityServices, eligible,
};
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::static_eligibility::tokens::{TokenPayload, count_for_pre_call_checks};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod counter;

use counter::{ProbeCounter, ProbeLimits, ProbeSupport};

const API_KEY: &str = "sk-probe";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
    #[serde(default)]
    pub base_model: Option<String>,
    #[serde(default)]
    pub max_input_tokens: Option<u64>,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub supports_web_search: Option<bool>,
    #[serde(default)]
    pub supported_environments: Option<Vec<String>>,
    #[serde(default)]
    pub order: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    pub model: String,
    #[serde(default)]
    pub specific_deployment: bool,
    #[serde(default)]
    pub messages: Option<Vec<Json>>,
    #[serde(default)]
    pub tools: Vec<Json>,
    #[serde(default)]
    pub response_format: bool,
    #[serde(default)]
    pub target_order: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub environment: String,
    pub enable_pre_call_checks: bool,
    #[serde(default)]
    pub drop_params: bool,
    pub models: Vec<Entry>,
    pub lookups: Vec<Lookup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Selected,
    BadRequest,
    RateLimited,
    ContextExceeded,
    ServiceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Found {
    pub outcome: Outcome,
    pub selected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub lookups: Vec<Found>,
}

fn model_info(entry: &Entry) -> JsonObject {
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(entry.id.clone()));
    if let Some(base_model) = &entry.base_model {
        info.insert("base_model".to_owned(), Json::Text(base_model.clone()));
    }
    if let Some(maximum) = entry.max_input_tokens {
        info.insert(
            "max_input_tokens".to_owned(),
            Json::Number(serde_json::Number::from(maximum)),
        );
    }
    if let Some(blocked) = entry.blocked {
        info.insert("blocked".to_owned(), Json::Bool(blocked));
    }
    if let Some(supports) = entry.supports_web_search {
        info.insert("supports_web_search".to_owned(), Json::Bool(supports));
    }
    if let Some(environments) = &entry.supported_environments {
        info.insert(
            "supported_environments".to_owned(),
            Json::List(
                environments
                    .iter()
                    .map(|name| Json::Text(name.clone()))
                    .collect(),
            ),
        );
    }
    info
}

// One registered deployment; None when Router would have skipped it at construction because of its environment.
fn spec(input: &Input, entry: &Entry) -> Result<Option<DeploymentSpec>, OperationError> {
    let info = model_info(entry);
    let active =
        model_info_is_active_for_environment(&info, Some(&input.environment)).map_err(|error| {
            OperationError::InvalidRequest {
                message: error.to_string(),
            }
        })?;
    if !active {
        return Ok(None);
    }
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(entry.model.clone()));
    params.insert("api_key".to_owned(), Json::Text(API_KEY.to_owned()));
    if let Some(order) = entry.order {
        params.insert(
            "order".to_owned(),
            Json::Number(serde_json::Number::from(order)),
        );
    }
    DeploymentSpec::new(
        entry.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .map(Some)
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

fn deployable<'entries, I>(input: &Input, entries: I) -> Result<Vec<DeploymentSpec>, OperationError>
where
    I: Iterator<Item = &'entries Entry>,
{
    let mut pool = Vec::new();
    for entry in entries {
        if let Some(built) = spec(input, entry)? {
            pool.push(built);
        }
    }
    Ok(pool)
}

// _common_checks_available_deployment as far as the probe cases reach: a group name, a deployment id, or a litellm model.
fn resolve(input: &Input, lookup: &Lookup) -> Result<Option<Candidates>, OperationError> {
    if lookup.specific_deployment {
        let pool = deployable(
            input,
            input
                .models
                .iter()
                .filter(|entry| entry.model == lookup.model),
        )?;
        return Ok(Some(Candidates::Pool(pool)));
    }
    let named: Vec<&Entry> = input
        .models
        .iter()
        .filter(|entry| entry.model_name == lookup.model)
        .collect();
    if !named.is_empty() {
        return Ok(Some(Candidates::Pool(deployable(
            input,
            named.into_iter(),
        )?)));
    }
    let Some(entry) = input.models.iter().find(|entry| entry.id == lookup.model) else {
        return Ok(None);
    };
    Ok(spec(input, entry)?.map(|built| Candidates::Single(Box::new(built))))
}

fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    let mut selected = Vec::new();
    for deployment in deployments {
        selected.push(deployment.id().unwrap_or_default().to_owned());
    }
    selected
}

async fn answer(
    input: &Input,
    lookup: &Lookup,
    counter: &ProbeCounter,
) -> Result<Found, OperationError> {
    let Some(candidates) = resolve(input, lookup)? else {
        return Ok(Found {
            outcome: Outcome::BadRequest,
            selected: Vec::new(),
        });
    };
    let mut facts = RequestFacts {
        messages: lookup.messages.clone(),
        system: None,
        tools: lookup.tools.clone(),
        response_format: lookup.response_format,
        target_order: lookup.target_order,
        input_token_count: None,
        compaction_pending: false,
        drop_params: input.drop_params,
    };
    if input.enable_pre_call_checks && facts.messages.is_some() {
        let payload = TokenPayload {
            messages: facts.messages.as_deref(),
            system: None,
            tools: &facts.tools,
        };
        facts.input_token_count = count_for_pre_call_checks(
            &lookup.model,
            candidates.deployments(),
            &payload,
            &ProbeLimits,
            counter,
        )
        .await;
    }
    let services = EligibilityServices {
        limits: &ProbeLimits,
        params: &ProbeSupport,
    };
    let outcome = match eligible(
        EligibilityInput {
            model: &lookup.model,
            candidates,
            facts: &facts,
            environment: Some(&input.environment),
            enable_pre_call_checks: input.enable_pre_call_checks,
        },
        &services,
    ) {
        Ok(EligibilityResult { deployments, .. }) if deployments.is_empty() => Found {
            outcome: Outcome::RateLimited,
            selected: Vec::new(),
        },
        Ok(EligibilityResult { deployments, .. }) => Found {
            outcome: Outcome::Selected,
            selected: ids(&deployments),
        },
        Err(EligibilityError::Blocked { .. }) => Found {
            outcome: Outcome::ServiceUnavailable,
            selected: Vec::new(),
        },
        Err(EligibilityError::ContextWindow(_error)) => Found {
            outcome: Outcome::ContextExceeded,
            selected: Vec::new(),
        },
        Err(EligibilityError::Environment(error)) => {
            return Err(OperationError::InvalidRequest {
                message: error.to_string(),
            });
        }
        Err(EligibilityError::WebSearch(error)) => {
            return Err(OperationError::InvalidRequest {
                message: error.to_string(),
            });
        }
    };
    Ok(outcome)
}

async fn execute(input: &Input) -> Result<Output, OperationError> {
    let counter = ProbeCounter::new()?;
    let mut lookups = Vec::new();
    for lookup in &input.lookups {
        lookups.push(answer(input, lookup, &counter).await?);
    }
    Ok(Output { lookups })
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let output = runtime.block_on(execute(input))?;
    Ok(ProbeOutput::StaticEligibility(output))
}
