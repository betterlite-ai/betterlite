use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};

use self::environment::{PARAMS_DEPTH, resolve_map, resolve_params};
use self::keys::{BASE_KEYS, PARAM_KEYS};
use self::opaque::{ExactValue, OpaqueParams};
use self::params::LitellmParams;
use self::records::Guardrail;
use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::guardrail_identity::item::GuardrailValue;

pub mod base;
pub mod content_filter;
pub mod decision_check;
pub mod decode;
pub mod environment;
pub mod error;
pub mod keys;
pub mod mode;
pub mod opaque;
pub mod params;
pub mod records;
pub mod routes;
mod tests;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<RawEntry>,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    guardrail_name: String,
    guardrail_id: Option<String>,
    guardrail_info: Option<OpaqueParams>,
    policy_template: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    litellm_params: KnownParams,
}

// only the keys this section reads go through the map, so serde_ignored still reports the rest as unsupported
#[derive(Debug, Default)]
struct KnownParams(BTreeMap<String, GuardrailValue>);

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for KnownParams {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(KnownParamsVisitor)
    }
}

struct KnownParamsVisitor;

// ast-grep-ignore: no-custom-serde
impl<'de> Visitor<'de> for KnownParamsVisitor {
    type Value = KnownParams;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a litellm_params mapping")
    }

    fn visit_map<A>(self, mut map: A) -> Result<KnownParams, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut known = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if BASE_KEYS.contains(&key.as_str()) || PARAM_KEYS.contains(&key.as_str()) {
                known.insert(key, map.next_value::<ExactValue>()?.0);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(KnownParams(known))
    }
}

#[derive(Debug, Clone)]
pub struct RejectedGuardrail {
    pub guardrail_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Guardrail>,
    pub rejected: Vec<RejectedGuardrail>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut settings = Settings::default();
        for entry in self.guardrails {
            let guardrail_info = entry
                .guardrail_info
                .map(|info| OpaqueParams(resolve_map(info.0, PARAMS_DEPTH, lookup)));
            let built = resolve_params(entry.litellm_params.0, lookup)
                .and_then(|params| LitellmParams::try_from(OpaqueParams(params)));
            match built {
                Ok(litellm_params) => settings.guardrails.push(Guardrail {
                    guardrail_id: entry.guardrail_id,
                    guardrail_name: entry.guardrail_name,
                    litellm_params,
                    guardrail_info,
                    policy_template: entry.policy_template,
                    created_at: entry.created_at,
                    updated_at: entry.updated_at,
                    team_id: None,
                }),
                Err(error) => settings.rejected.push(RejectedGuardrail {
                    guardrail_name: entry.guardrail_name,
                    reason: error.to_string(),
                }),
            }
        }
        Ok(settings)
    }
}
