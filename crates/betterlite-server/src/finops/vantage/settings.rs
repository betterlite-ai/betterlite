use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::finops::focus::destinations::factory::resolve::DestinationOverrides;
use betterlite_logging::finops::vantage::settings::DEFAULT_BASE_URL;
use secrecy::{ExposeSecret as _, SecretString};

pub(super) const PROVIDER: &str = "vantage";
pub(super) const EXPORT_FORMAT: &str = "csv";
pub(super) const EXPORT_PREFIX: &str = "vantage_exports";
const DEFAULT_FREQUENCY: &str = "hourly";

const API_KEY_VARIABLE: &str = "VANTAGE_API_KEY";
const INTEGRATION_TOKEN_VARIABLE: &str = "VANTAGE_INTEGRATION_TOKEN";
const BASE_URL_VARIABLE: &str = "VANTAGE_BASE_URL";
const FREQUENCY_VARIABLE: &str = "VANTAGE_EXPORT_FREQUENCY";
const INTERVAL_VARIABLE: &str = "VANTAGE_EXPORT_INTERVAL_SECONDS";

// VantageLogger.__init__'s keyword arguments; cron_offset_minute arrives through the source's **kwargs.
#[derive(Debug, Clone, Default)]
pub struct VantageLoggerParams {
    pub api_key: Option<SecretString>,
    pub integration_token: Option<SecretString>,
    pub base_url: Option<String>,
    pub frequency: Option<String>,
    pub interval_seconds: Option<i64>,
    pub cron_offset_minute: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct VantageSettings {
    pub api_key: Option<SecretString>,
    pub integration_token: Option<SecretString>,
    pub base_url: String,
    pub frequency: String,
    pub interval_seconds: Option<i64>,
}

impl VantageSettings {
    // the source's __init__ order: the parameter, then the environment, then the default
    #[must_use]
    pub fn resolve<L>(params: &VantageLoggerParams, environment: &L) -> Self
    where
        L: EnvironmentLookup + ?Sized,
    {
        Self {
            api_key: resolved_secret(params.api_key.as_ref(), API_KEY_VARIABLE, environment),
            integration_token: resolved_secret(
                params.integration_token.as_ref(),
                INTEGRATION_TOKEN_VARIABLE,
                environment,
            ),
            base_url: resolved_text(params.base_url.as_deref(), BASE_URL_VARIABLE, environment)
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            frequency: resolved_text(params.frequency.as_deref(), FREQUENCY_VARIABLE, environment)
                .unwrap_or_else(|| DEFAULT_FREQUENCY.to_owned())
                .to_lowercase(),
            interval_seconds: resolved_interval(params.interval_seconds, environment),
        }
    }

    // the parent engine's destination config: base_url always, each credential only when it is set
    #[must_use]
    pub fn destination_overrides(&self) -> DestinationOverrides {
        DestinationOverrides {
            bucket_name: None,
            region_name: None,
            endpoint_url: None,
            aws_access_key_id: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            api_key: self.api_key.clone(),
            integration_token: self.integration_token.clone(),
            base_url: Some(self.base_url.clone()),
            service_account_json: None,
            api_endpoint: None,
            connection_id: None,
        }
    }
}

// the source's masked form: the first four characters plus ***, and *** for anything shorter
#[must_use]
pub fn masked_integration_token(token: Option<&SecretString>) -> String {
    let Some(secret) = token else {
        return "***".to_owned();
    };
    let exposed = secret.expose_secret();
    if exposed.chars().count() <= 4 {
        return "***".to_owned();
    }
    let visible: String = exposed.chars().take(4).collect();
    format!("{visible}***")
}

// a parameter is used as given, an empty one included; the environment answers only when it is absent
fn resolved_text<L>(parameter: Option<&str>, variable: &str, environment: &L) -> Option<String>
where
    L: EnvironmentLookup + ?Sized,
{
    parameter.map(str::to_owned).or_else(|| {
        environment
            .lookup(variable)
            .filter(|value| !value.is_empty())
    })
}

fn resolved_secret<L>(
    parameter: Option<&SecretString>,
    variable: &str,
    environment: &L,
) -> Option<SecretString>
where
    L: EnvironmentLookup + ?Sized,
{
    parameter
        .cloned()
        .or_else(|| resolved_text(None, variable, environment).map(SecretString::from))
}

fn resolved_interval<L>(parameter: Option<i64>, environment: &L) -> Option<i64>
where
    L: EnvironmentLookup + ?Sized,
{
    match parameter {
        Some(seconds) => Some(seconds),
        None => {
            let raw = environment.lookup(INTERVAL_VARIABLE)?;
            betterlite_env::number::<i64>(INTERVAL_VARIABLE, &raw).unwrap_or_else(|_error| {
                tracing::warn!("Invalid VANTAGE_EXPORT_INTERVAL_SECONDS value: {raw}, ignoring");
                None
            })
        }
    }
}
