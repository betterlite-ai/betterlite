use std::collections::HashMap;

use aws_config::{BehaviorVersion, Region, retry::RetryConfig, timeout::TimeoutConfig};
use aws_sdk_sts::{Client, config::Builder};
use aws_smithy_types::error::display::DisplayErrorContext;
use aws_types::os_shim_internal::Env;

use crate::aws_settings::sources::NativeSources;

pub mod error;
pub mod expiry;
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub struct ArnIdentity<'arn> {
    pub partition: &'arn str,
    pub account: &'arn str,
    pub role: &'arn str,
}

#[must_use]
pub fn parse_role_arn(arn: &str) -> Option<ArnIdentity<'_>> {
    // splitn keeps colons inside the resource, which LiteLLM rejoins after a full split
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    let ["arn", partition, _service, _region, account, resource] = parts.as_slice() else {
        return None;
    };
    let role = if let Some(path) = resource.strip_prefix("role/") {
        path.rsplit_once('/').map_or(path, |(_, name)| name)
    } else {
        let session = resource.strip_prefix("assumed-role/")?;
        session.split_once('/').map_or(session, |(name, _)| name)
    };
    Some(ArnIdentity {
        partition,
        account,
        role,
    })
}

async fn default_sts_client(native: &NativeSources) -> Client {
    let shared = aws_config::defaults(BehaviorVersion::latest())
        .env(Env::from(HashMap::from_iter(native.environment.clone())))
        .fs(native.filesystem.clone())
        .http_client(native.client.clone())
        .time_source(native.clock.clone())
        .sleep_impl(native.sleep.clone())
        .retry_config(RetryConfig::disabled())
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(native.timeout)
                .build(),
        )
        .load()
        .await;
    // boto3.client("sts") without a region talks to the global endpoint
    if shared.region().is_none() {
        let global = Builder::from(&shared).region(Region::from_static("aws-global"));
        return Client::from_conf(global.build());
    }
    Client::new(&shared)
}

pub async fn is_already_running_as_role(aws_role_name: &str, native: &NativeSources) -> bool {
    let Some(target) = parse_role_arn(aws_role_name) else {
        return false;
    };
    let ambient = |name| {
        native
            .environment
            .get(name)
            .filter(|value| !value.is_empty())
    };
    if let (Some(current_role), Some(_token_file)) = (
        ambient("AWS_ROLE_ARN"),
        ambient("AWS_WEB_IDENTITY_TOKEN_FILE"),
    ) {
        return current_role == aws_role_name;
    }
    match default_sts_client(native)
        .await
        .get_caller_identity()
        .send()
        .await
    {
        Ok(identity) => {
            let matched = parse_role_arn(identity.arn().unwrap_or_default())
                .is_some_and(|caller| caller == target);
            if matched {
                tracing::debug!("Current identity already matches target role: {aws_role_name}");
            }
            matched
        }
        Err(error) => {
            tracing::debug!(
                "Could not determine current role identity: {}",
                DisplayErrorContext(&error)
            );
            false
        }
    }
}
