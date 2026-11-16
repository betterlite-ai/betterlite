use betterlite_config::loader::startup::process_environment;
use betterlite_env::flag;
use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup;
use serde_json::{Map, Value};

mod tests;

// The REDIS_<UPPER NAME> variables of the redis client arguments the settings page renders, names LiteLLM's _get_redis_env_kwarg_mapping gives.
pub const ENVIRONMENT_FIELDS: [(&str, &str); 17] = [
    ("REDIS_AWS_IAM_AUTH", "aws_iam_auth"),
    ("REDIS_AWS_IAM_CACHE_NAME", "aws_iam_cache_name"),
    ("REDIS_AWS_IAM_REGION", "aws_iam_region"),
    ("REDIS_AWS_IAM_SERVERLESS", "aws_iam_serverless"),
    ("REDIS_AWS_IAM_USER_NAME", "aws_iam_user_name"),
    ("REDIS_DB", "db"),
    ("REDIS_GCP_SERVICE_ACCOUNT", "gcp_service_account"),
    ("REDIS_GCP_SSL_CA_CERTS", "gcp_ssl_ca_certs"),
    ("REDIS_HOST", "host"),
    ("REDIS_MAX_CONNECTIONS", "max_connections"),
    ("REDIS_PASSWORD", "password"),
    ("REDIS_PORT", "port"),
    ("REDIS_SSL", "ssl"),
    ("REDIS_SSL_CERT_REQS", "ssl_cert_reqs"),
    ("REDIS_SSL_CHECK_HOSTNAME", "ssl_check_hostname"),
    ("REDIS_URL", "url"),
    ("REDIS_USERNAME", "username"),
];

// get_secret reads os.environ on every call, so the route sees an environment changed since the last request.
pub struct ProcessEnvironment;

impl EnvironmentLookup for ProcessEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        process_environment().get(name).cloned()
    }
}

// The catalog fields of Boolean type; a REDIS_ variable for one reads as a bool, the rest stay raw strings.
const BOOLEAN_FIELDS: [&str; 4] = [
    "aws_iam_auth",
    "aws_iam_serverless",
    "ssl",
    "ssl_check_hostname",
];

// The environment fills only what the stored settings leave out; a stored key present even with null keeps its value.
#[must_use]
pub fn overlay_environment(
    stored: &Map<String, Value>,
    environment: &dyn EnvironmentLookup,
) -> Map<String, Value> {
    let mut effective = Map::new();
    for (name, field) in ENVIRONMENT_FIELDS {
        let Some(raw) = environment.lookup(name) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        let value = match flag(name, &raw) {
            Ok(Some(parsed)) if BOOLEAN_FIELDS.contains(&field) => Value::Bool(parsed),
            _ => Value::String(raw),
        };
        effective.insert(field.to_owned(), value);
    }
    if effective.is_empty() {
        return stored.clone();
    }
    for (key, value) in stored {
        effective.insert(key.clone(), value.clone());
    }
    if !effective.contains_key("type") {
        effective.insert("type".to_owned(), Value::String("redis".to_owned()));
    }
    effective
}
