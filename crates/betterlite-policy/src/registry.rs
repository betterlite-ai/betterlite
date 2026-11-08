mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::pipeline::{GuardrailPipeline, PipelineError, PipelineMode, PipelineStep};
use crate::values::{Policy, PolicyCondition, PolicyGuardrails};

pub const POLICY_VERSION_ID_PREFIX: &str = "policy_";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    Config,
    #[default]
    Db,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyRegistry {
    pub policies: BTreeMap<String, Policy>,
    pub config_policies: BTreeMap<String, Policy>,
    pub sources: BTreeMap<String, PolicySource>,
    pub policies_by_id: BTreeMap<String, (String, Policy)>,
    pub initialized: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicy {
    pub inherit: Option<String>,
    pub description: Option<String>,
    pub guardrails: Option<RawGuardrails>,
    pub condition: Option<PolicyCondition>,
    pub pipeline: Option<RawPipeline>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawGuardrails {
    Lists(RawGuardrailLists),
    Add(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawGuardrailLists {
    pub add: Option<Vec<String>>,
    pub remove: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPipeline {
    #[serde(default = "default_mode")]
    pub mode: PipelineMode,
    #[serde(default)]
    pub steps: Vec<PipelineStep>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Invalid policy '{policy}': {source}")]
    Policy {
        policy: String,
        #[source]
        source: PipelineError,
    },
}

const fn default_mode() -> PipelineMode {
    PipelineMode::PreCall
}

pub fn parse_pipeline(data: RawPipeline) -> Result<GuardrailPipeline, PipelineError> {
    if data.steps.is_empty() {
        return Err(PipelineError::NoSteps);
    }
    Ok(GuardrailPipeline {
        mode: data.mode,
        steps: data.steps,
    })
}

pub fn parse_policy(policy_name: &str, data: RawPolicy) -> Result<Policy, RegistryError> {
    let guardrails = match data.guardrails {
        Some(RawGuardrails::Lists(lists)) => PolicyGuardrails {
            add: lists.add,
            remove: lists.remove,
        },
        Some(RawGuardrails::Add(add)) => PolicyGuardrails {
            add: Some(add).filter(|names| !names.is_empty()),
            remove: None,
        },
        None => PolicyGuardrails::default(),
    };
    let pipeline = data
        .pipeline
        .map(parse_pipeline)
        .transpose()
        .map_err(|source| RegistryError::Policy {
            policy: policy_name.to_owned(),
            source,
        })?;
    Ok(Policy {
        inherit: data.inherit,
        description: data.description,
        guardrails,
        condition: data.condition,
        pipeline,
    })
}

impl PolicyRegistry {
    pub fn load_policies(
        &mut self,
        policies_config: BTreeMap<String, RawPolicy>,
    ) -> Result<(), RegistryError> {
        let mut policies = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for (policy_name, data) in policies_config {
            let policy = parse_policy(&policy_name, data)?;
            tracing::debug!("Loaded policy: {policy_name}");
            policies.insert(policy_name.clone(), policy);
            sources.insert(policy_name, PolicySource::Config);
        }
        self.config_policies = policies.clone();
        self.policies = policies;
        self.sources = sources;
        self.policies_by_id = BTreeMap::new();
        self.initialized = true;
        tracing::info!("Loaded {} policies", self.policies.len());
        Ok(())
    }

    pub fn add_policy(&mut self, policy_name: &str, policy: Policy, source: PolicySource) {
        if source == PolicySource::Config {
            self.config_policies
                .insert(policy_name.to_owned(), policy.clone());
        }
        self.policies.insert(policy_name.to_owned(), policy);
        self.sources.insert(policy_name.to_owned(), source);
        self.initialized = true;
        tracing::debug!("Added/updated policy: {policy_name}");
    }

    pub fn remove_policy(&mut self, policy_name: &str) -> bool {
        if !self.policies.contains_key(policy_name) {
            return false;
        }
        if let Some(config_fallback) = self.config_policies.get(policy_name).cloned() {
            self.policies
                .insert(policy_name.to_owned(), config_fallback);
            self.sources
                .insert(policy_name.to_owned(), PolicySource::Config);
            tracing::debug!("Removed policy: {policy_name}; restored config-defined version");
            return true;
        }
        self.policies.remove(policy_name);
        self.sources.remove(policy_name);
        tracing::debug!("Removed policy: {policy_name}");
        true
    }
}
