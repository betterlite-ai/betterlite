use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, resolve_number};

use super::AllowedFailsPolicy;

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawPolicy {
    #[serde(rename = "ContentPolicyViolationErrorAllowedFails")]
    content_policy_violation: Option<RawNumber>,
    #[serde(rename = "BadRequestErrorAllowedFails")]
    bad_request: Option<RawNumber>,
    #[serde(rename = "AuthenticationErrorAllowedFails")]
    authentication: Option<RawNumber>,
    #[serde(rename = "TimeoutErrorAllowedFails")]
    timeout: Option<RawNumber>,
    #[serde(rename = "RateLimitErrorAllowedFails")]
    rate_limit: Option<RawNumber>,
    #[serde(rename = "InternalServerErrorAllowedFails")]
    internal_server: Option<RawNumber>,
    #[serde(rename = "ServiceUnavailableErrorAllowedFails")]
    service_unavailable: Option<RawNumber>,
    #[serde(rename = "BadGatewayErrorAllowedFails")]
    bad_gateway: Option<RawNumber>,
    #[serde(rename = "NotFoundErrorAllowedFails")]
    not_found: Option<RawNumber>,
}

impl RawPolicy {
    pub(super) fn resolve<L>(
        self,
        prefix: &str,
        lookup: &L,
    ) -> Result<AllowedFailsPolicy, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let entries = [
            (
                "ContentPolicyViolationErrorAllowedFails",
                self.content_policy_violation,
            ),
            ("BadRequestErrorAllowedFails", self.bad_request),
            ("AuthenticationErrorAllowedFails", self.authentication),
            ("TimeoutErrorAllowedFails", self.timeout),
            ("RateLimitErrorAllowedFails", self.rate_limit),
            ("InternalServerErrorAllowedFails", self.internal_server),
            (
                "ServiceUnavailableErrorAllowedFails",
                self.service_unavailable,
            ),
            ("BadGatewayErrorAllowedFails", self.bad_gateway),
            ("NotFoundErrorAllowedFails", self.not_found),
        ];
        let mut resolved = [None; 9];
        for (slot, (name, value)) in resolved.iter_mut().zip(entries) {
            *slot = policy_value(value, prefix, name, lookup)?;
        }
        Ok(AllowedFailsPolicy::from(resolved))
    }
}

impl From<[Option<u32>; 9]> for AllowedFailsPolicy {
    fn from(values: [Option<u32>; 9]) -> Self {
        let [
            content_policy_violation,
            bad_request,
            authentication,
            timeout,
            rate_limit,
            internal_server,
            service_unavailable,
            bad_gateway,
            not_found,
        ] = values;
        Self {
            content_policy_violation,
            bad_request,
            authentication,
            timeout,
            rate_limit,
            internal_server,
            service_unavailable,
            bad_gateway,
            not_found,
        }
    }
}

fn policy_value<L>(
    raw: Option<RawNumber>,
    prefix: &str,
    name: &str,
    lookup: &L,
) -> Result<Option<u32>, ConfigError>
where
    L: EnvironmentLookup,
{
    raw.map_or(Ok(None), |value| {
        let path = format!("{prefix}.{name}");
        resolve_number(Some(value), &path, lookup, parse_integer)
    })
}
