use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

mod tests;

const DEFAULT_INITIAL_RETRY_DELAY: f64 = 0.5;
const DEFAULT_MAX_RETRY_DELAY: f64 = 8.0;
const DEFAULT_JITTER: f64 = 0.75;
// Router.__init__ falls back to openai.DEFAULT_MAX_RETRIES, which the pinned openai sets to 2
const DEFAULT_NUM_RETRIES: i64 = 2;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    retry_policy: Option<RawPolicy>,
    model_group_retry_policy: Option<BTreeMap<String, RawPolicy>>,
    num_retries: Option<RawNumber>,
    retry_after: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    num_retries: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    #[serde(rename = "BadRequestErrorRetries")]
    bad_request: Option<RawNumber>,
    #[serde(rename = "AuthenticationErrorRetries")]
    authentication: Option<RawNumber>,
    #[serde(rename = "TimeoutErrorRetries")]
    timeout: Option<RawNumber>,
    #[serde(rename = "RateLimitErrorRetries")]
    rate_limit: Option<RawNumber>,
    #[serde(rename = "ContentPolicyViolationErrorRetries")]
    content_policy: Option<RawNumber>,
    #[serde(rename = "InternalServerErrorRetries")]
    internal_server: Option<RawNumber>,
    #[serde(rename = "ServiceUnavailableErrorRetries")]
    service_unavailable: Option<RawNumber>,
    #[serde(rename = "NotFoundErrorRetries")]
    not_found: Option<RawNumber>,
    #[serde(rename = "DefaultRetries")]
    fallback: Option<RawNumber>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryPolicySettings {
    pub bad_request: Option<i64>,
    pub authentication: Option<i64>,
    pub timeout: Option<i64>,
    pub rate_limit: Option<i64>,
    pub content_policy: Option<i64>,
    pub internal_server: Option<i64>,
    pub service_unavailable: Option<i64>,
    pub not_found: Option<i64>,
    pub fallback: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub retry_policy: Option<RetryPolicySettings>,
    pub model_group_retry_policy: BTreeMap<String, RetryPolicySettings>,
    pub num_retries: i64,
    pub retry_after: f64,
    pub initial_retry_delay: f64,
    pub max_retry_delay: f64,
    pub jitter: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            retry_policy: None,
            model_group_retry_policy: BTreeMap::new(),
            num_retries: DEFAULT_NUM_RETRIES,
            retry_after: 0.0,
            initial_retry_delay: DEFAULT_INITIAL_RETRY_DELAY,
            max_retry_delay: DEFAULT_MAX_RETRY_DELAY,
            jitter: DEFAULT_JITTER,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Self {
            router_settings: router,
            litellm_settings: litellm,
        } = self;
        let num_retries = resolve_retries(router.num_retries, litellm.num_retries, lookup)?;
        let initial_retry_delay = {
            let text = lookup.lookup("INITIAL_RETRY_DELAY").unwrap_or_default();
            betterlite_env::number::<f64>("INITIAL_RETRY_DELAY", &text)?
                .unwrap_or(DEFAULT_INITIAL_RETRY_DELAY)
        };
        let max_retry_delay = {
            let text = lookup.lookup("MAX_RETRY_DELAY").unwrap_or_default();
            betterlite_env::number::<f64>("MAX_RETRY_DELAY", &text)?
                .unwrap_or(DEFAULT_MAX_RETRY_DELAY)
        };
        let jitter = {
            let text = lookup.lookup("JITTER").unwrap_or_default();
            betterlite_env::number::<f64>("JITTER", &text)?.unwrap_or(DEFAULT_JITTER)
        };
        Ok(Settings {
            retry_policy: router
                .retry_policy
                .map(|policy| policy.resolve("router_settings.retry_policy", lookup))
                .transpose()?,
            model_group_retry_policy: group_policies(router.model_group_retry_policy, lookup)?,
            num_retries,
            retry_after: resolve_number(
                router.retry_after,
                "router_settings.retry_after",
                lookup,
                parse_float,
            )?
            .unwrap_or(0.0),
            initial_retry_delay,
            max_retry_delay,
            jitter,
        })
    }
}

impl RawPolicy {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<RetryPolicySettings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(RetryPolicySettings {
            bad_request: retries(self.bad_request, path, "BadRequestErrorRetries", lookup)?,
            authentication: retries(
                self.authentication,
                path,
                "AuthenticationErrorRetries",
                lookup,
            )?,
            timeout: retries(self.timeout, path, "TimeoutErrorRetries", lookup)?,
            rate_limit: retries(self.rate_limit, path, "RateLimitErrorRetries", lookup)?,
            content_policy: retries(
                self.content_policy,
                path,
                "ContentPolicyViolationErrorRetries",
                lookup,
            )?,
            internal_server: retries(
                self.internal_server,
                path,
                "InternalServerErrorRetries",
                lookup,
            )?,
            service_unavailable: retries(
                self.service_unavailable,
                path,
                "ServiceUnavailableErrorRetries",
                lookup,
            )?,
            not_found: retries(self.not_found, path, "NotFoundErrorRetries", lookup)?,
            fallback: retries(self.fallback, path, "DefaultRetries", lookup)?,
        })
    }
}

fn retries<L>(
    value: Option<RawNumber>,
    path: &str,
    name: &str,
    lookup: &L,
) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    value.map_or(Ok(None), |number| {
        resolve_number(
            Some(number),
            &format!("{path}.{name}"),
            lookup,
            parse_retries,
        )
    })
}

fn resolve_retries<L>(
    router: Option<RawNumber>,
    litellm: Option<RawNumber>,
    lookup: &L,
) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(value) =
        resolve_number(router, "router_settings.num_retries", lookup, parse_retries)?
    {
        return Ok(value);
    }
    let fallback = resolve_number(
        litellm,
        "litellm_settings.num_retries",
        lookup,
        parse_retries,
    )?;
    Ok(fallback.unwrap_or(DEFAULT_NUM_RETRIES))
}

fn group_policies<L>(
    raw: Option<BTreeMap<String, RawPolicy>>,
    lookup: &L,
) -> Result<BTreeMap<String, RetryPolicySettings>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut policies = BTreeMap::new();
    for (group, policy) in raw.unwrap_or_default() {
        let path = format!("router_settings.model_group_retry_policy.{group}");
        policies.insert(group, policy.resolve(&path, lookup)?);
    }
    Ok(policies)
}

fn parse_retries(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
