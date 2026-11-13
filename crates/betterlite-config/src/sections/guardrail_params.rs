use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};

use self::environment::{PARAMS_DEPTH, resolve_map, resolve_params, resolve_text};
use self::error::GuardrailParamsError;
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
    created_at: Option<String>,
    updated_at: Option<String>,
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
            settings.record(entry, lookup);
        }
        Ok(settings)
    }
}

impl Settings {
    fn record<L>(&mut self, entry: RawEntry, lookup: &L)
    where
        L: EnvironmentLookup,
    {
        let written = entry.guardrail_name.clone();
        match entry.build(lookup) {
            Ok(guardrail) => self.guardrails.push(guardrail),
            Err(error) => {
                let shown = resolve_text(written.clone(), lookup).unwrap_or(written);
                self.rejected.push(RejectedGuardrail {
                    guardrail_name: shown,
                    reason: error.to_string(),
                });
            }
        }
    }
}

impl RawEntry {
    fn build<L>(self, lookup: &L) -> Result<Guardrail, GuardrailParamsError>
    where
        L: EnvironmentLookup,
    {
        let Some(guardrail_name) = resolve_text(self.guardrail_name, lookup) else {
            return Err(GuardrailParamsError::MissingField {
                field: "guardrail_name".to_owned(),
            });
        };
        let params = resolve_params(self.litellm_params.0, lookup)?;
        Ok(Guardrail {
            guardrail_id: self.guardrail_id.and_then(|id| resolve_text(id, lookup)),
            guardrail_name,
            litellm_params: LitellmParams::try_from(OpaqueParams(params))?,
            guardrail_info: self
                .guardrail_info
                .map(|info| OpaqueParams(resolve_map(info.0, PARAMS_DEPTH, lookup))),
            policy_template: self
                .policy_template
                .and_then(|template| resolve_text(template, lookup)),
            created_at: resolve_time("created_at", self.created_at, lookup)?,
            updated_at: resolve_time("updated_at", self.updated_at, lookup)?,
            team_id: None,
        })
    }
}

fn resolve_time<L>(
    field: &str,
    value: Option<String>,
    lookup: &L,
) -> Result<Option<DateTime<Utc>>, GuardrailParamsError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value.and_then(|text| resolve_text(text, lookup)) else {
        return Ok(None);
    };
    text.parse()
        .map(Some)
        .map_err(|_unparsed| GuardrailParamsError::InvalidField {
            field: field.to_owned(),
        })
}
