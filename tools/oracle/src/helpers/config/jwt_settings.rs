use std::collections::BTreeMap;

use betterlite_config::loader::parse_view;
use betterlite_config::sections::jwt_settings::Settings;
use betterlite_config::sections::jwt_settings::construct::{Context, construct};
use betterlite_config::sections::jwt_settings::error::{ALLOWED_KEYS, JwtSettingsError, Location};
use betterlite_config::sections::jwt_settings::raw::RawJwtAuth;
use betterlite_config::sections::jwt_settings::records::{
    IssuerConfig, UnregisteredClientBehavior,
};
use betterlite_config::sections::jwt_settings::validator::UnregisteredLoader;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    // passthrough: the litellm_jwtauth mapping exactly as the case writes it
    settings: Map<String, Value>,
    references: BTreeMap<String, String>,
    environment: BTreeMap<String, String>,
    issuers: Vec<Option<String>>,
}

#[derive(Serialize)]
struct Output {
    outcomes: Vec<Outcome>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Rejected {
    Validation,
    ValueError,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Outcome {
    Accepted {
        accepted: Settings,
        virtual_key_mapping_configured: bool,
        probes: Vec<Probe>,
    },
    Field {
        rejected: Rejected,
        location: Location,
        constraint: Option<String>,
    },
    Keys {
        rejected: Rejected,
        invalid: Vec<String>,
        allowed: Vec<&'static str>,
    },
    Message {
        rejected: Rejected,
        message: String,
    },
}

#[derive(Serialize)]
struct Probe {
    issuer: Option<String>,
    issuer_config: Option<IssuerConfig>,
    virtual_key_claim_field: Option<String>,
    unregistered_jwt_client_behavior: UnregisteredClientBehavior,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes = input
            .scenarios
            .iter()
            .map(outcome)
            .collect::<Result<Vec<Outcome>, Box<HelperError>>>()?;
        serde_json::to_value(Output { outcomes }).or_value_error()
    }
}

// a reference becomes the os.environ/ text the config file holds, and the case environment resolves it
fn outcome(scenario: &Scenario) -> Result<Outcome, Box<HelperError>> {
    let mut settings = scenario.settings.clone();
    for (field, variable) in &scenario.references {
        settings.insert(
            field.clone(),
            Value::String(format!("os.environ/{variable}")),
        );
    }
    // JSON text is YAML, so the section reads the mapping through the config loader's own parser
    let document = serde_json::to_string(&settings).or_value_error()?;
    let (raw, _ignored) = parse_view::<RawJwtAuth>(&document).or_config_error()?;
    let context = Context {
        loader: &UnregisteredLoader,
        config_file_path: None,
        lookup: &scenario.environment,
    };
    match construct(raw, &context) {
        Ok(built) => Ok(accepted(built, &scenario.issuers)),
        Err(error) => rejected(&error),
    }
}

fn accepted(built: Settings, issuers: &[Option<String>]) -> Outcome {
    let probes = issuers
        .iter()
        .map(|issuer| Probe {
            issuer: issuer.clone(),
            issuer_config: built.issuer_config(issuer.as_deref()).cloned(),
            virtual_key_claim_field: built
                .virtual_key_claim_field(issuer.as_deref())
                .map(str::to_owned),
            unregistered_jwt_client_behavior: built
                .unregistered_jwt_client_behavior(issuer.as_deref()),
        })
        .collect();
    Outcome::Accepted {
        virtual_key_mapping_configured: built.is_virtual_key_mapping_configured(),
        probes,
        accepted: built,
    }
}

// pydantic reports a field's location, and the issuer validator's text, as a ValidationError
fn rejected(error: &JwtSettingsError) -> Result<Outcome, Box<HelperError>> {
    match error {
        JwtSettingsError::Invalid { location } => Ok(Outcome::Field {
            rejected: Rejected::Validation,
            location: location.clone(),
            constraint: None,
        }),
        JwtSettingsError::AudienceMissing { location, .. }
        | JwtSettingsError::AudienceConflict { location, .. } => Ok(Outcome::Field {
            rejected: Rejected::Validation,
            location: location.clone(),
            constraint: Some(error.to_string()),
        }),
        JwtSettingsError::UnknownKeys { invalid } => Ok(Outcome::Keys {
            rejected: Rejected::ValueError,
            invalid: invalid.clone(),
            allowed: Vec::from(ALLOWED_KEYS),
        }),
        JwtSettingsError::RolesWithoutPair
        | JwtSettingsError::ObjectIdWithoutRoleMappings
        | JwtSettingsError::ScopeMappingsWithoutEnforcement => Ok(Outcome::Message {
            rejected: Rejected::ValueError,
            message: error.to_string(),
        }),
        JwtSettingsError::ValidatorAnnotation { .. }
        | JwtSettingsError::ValidatorLoad { .. }
        | JwtSettingsError::MissingEnvironment { .. }
        | JwtSettingsError::Environment { .. } => Err::<Outcome, _>(error).or_config_error(),
    }
}
