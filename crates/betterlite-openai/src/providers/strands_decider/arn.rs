use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use sha2::{Digest as _, Sha256};
use url::Url;

use betterlite_context::aws_partition::get_aws_partition;

const ARN_SCHEME: &str = "arn";
const PARTITION_ROOT: &str = "aws";
const SERVICE: &str = "bedrock-agentcore";
const RESOURCE: &str = "runtime/";
const SESSION_PREFIX: &str = "litellm-decider-";
const RUNTIMES_OPEN: &str = "/runtimes/";
const RUNTIMES_CLOSE: &str = "/invocations";
const HOST_PREFIX: &str = "bedrock-agentcore";
const ACCOUNT_DIGITS: usize = 12;

// Python's urllib.parse.quote(arn, safe="") leaves only letters, digits and _.-~ alone
const ARN_SAFE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCoreRuntime {
    pub arn: String,
    pub region: String,
}

impl AgentCoreRuntime {
    #[must_use]
    pub fn invocations_url(&self) -> String {
        let partition = get_aws_partition(Some(&self.region));
        let host = format!("{HOST_PREFIX}.{}.{}", self.region, partition.dns_suffix);
        let encoded = utf8_percent_encode(&self.arn, &ARN_SAFE);
        format!("https://{host}{RUNTIMES_OPEN}{encoded}{RUNTIMES_CLOSE}")
    }

    #[must_use]
    pub fn default_session_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.arn.as_bytes());
        format!("{SESSION_PREFIX}{}", hex::encode(hasher.finalize()))
    }
}

#[must_use]
pub fn agentcore_runtime(api_base: &str) -> Option<AgentCoreRuntime> {
    let region = runtime_region(api_base)?;
    Some(AgentCoreRuntime {
        arn: api_base.to_owned(),
        region: region.to_owned(),
    })
}

#[must_use]
pub fn invoked_runtime(url: &str) -> Option<AgentCoreRuntime> {
    let parsed = Url::parse(url).ok()?;
    let encoded = parsed
        .path()
        .strip_prefix(RUNTIMES_OPEN)?
        .strip_suffix(RUNTIMES_CLOSE)?;
    let decoded = percent_decode_str(encoded).decode_utf8().ok()?;
    let runtime = agentcore_runtime(decoded.as_ref())?;
    (runtime.invocations_url() == parsed.as_str()).then_some(runtime)
}

fn runtime_region(arn: &str) -> Option<&str> {
    let mut fields = arn.split(':');
    let scheme = fields.next()?;
    let partition = fields.next()?;
    let service = fields.next()?;
    let region = fields.next()?;
    let account = fields.next()?;
    let resource = fields.next()?;
    if fields.next().is_some()
        || scheme != ARN_SCHEME
        || !is_partition(partition)
        || service != SERVICE
        || !is_region(region)
        || account.len() != ACCOUNT_DIGITS
        || !account.bytes().all(|byte| byte.is_ascii_digit())
        || !is_runtime_resource(resource)
    {
        return None;
    }
    Some(region)
}

fn is_partition(partition: &str) -> bool {
    let Some(suffix) = partition.strip_prefix(PARTITION_ROOT) else {
        return false;
    };
    suffix.is_empty()
        || (suffix.starts_with('-')
            && !suffix.ends_with('-')
            && !suffix.contains("--")
            && suffix
                .bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_lowercase()))
}

fn is_region(region: &str) -> bool {
    !region.is_empty()
        && region
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_runtime_resource(resource: &str) -> bool {
    resource.strip_prefix(RESOURCE).is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}
