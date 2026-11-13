use betterlite_config::sections::langfuse_settings::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::langfuse_settings::export::{flush_at, sample_rate};

use super::{ChannelSpec, LangfuseChannelRequest, TracingKey};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SampleRateKey(u64);

impl From<f64> for SampleRateKey {
    fn from(rate: f64) -> Self {
        if rate == 0.0_f64 {
            return Self(0.0_f64.to_bits());
        }
        Self(rate.to_bits())
    }
}

pub(super) fn channel_spec<'spec>(
    settings: &Settings,
    request: &'spec LangfuseChannelRequest,
) -> ChannelSpec<'spec> {
    ChannelSpec {
        environment: request.environment.as_deref(),
        release: request.release.as_deref(),
        sample_rate: sample_rate(settings),
        flush_interval_millis: request.flush_interval.saturating_mul(1_000),
        flush_at: flush_at(settings),
    }
}

pub(super) fn tracing_key(request: &LangfuseChannelRequest, spec: &ChannelSpec<'_>) -> TracingKey {
    TracingKey {
        public_key: request.public_key.clone(),
        secret_key_digest: secret_digest(&request.secret_key),
        base_url: request.base_url.clone(),
        environment: spec.environment.map(str::to_owned),
        release: spec.release.map(str::to_owned),
        sample_rate: SampleRateKey::from(spec.sample_rate),
        flush_at: spec.flush_at,
        flush_interval_millis: spec.flush_interval_millis,
        mock_mode: request.mock_mode,
    }
}

fn secret_digest(secret: &SecretString) -> [u8; 32] {
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(Sha256::digest(secret.expose_secret().as_bytes()).as_slice());
    digest
}
