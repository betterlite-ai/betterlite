use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::image_call::transport::{ClientKind, ImageRequest, SuppliedClient};
use betterlite_bedrock::image_edits::plan::{EditCall, EditEnvironment, EditImage};
use betterlite_openai::api::image_contracts::records::ImageParams;
use secrecy::SecretString;

use super::{Client, Factory, Source};
use crate::other_image_edits::input::{Probe, Supplied};

// Every probe signs at one instant, so the transport headers stay the same from run to run.
pub(crate) const NOW_SECONDS: i64 = 1_700_000_000;
const NOW_EPOCH_SECONDS: u64 = 1_700_000_000;

pub(crate) struct Harness<'probe> {
    probe: &'probe Probe,
    params: ImageParams,
    images: Vec<EditImage>,
    api_key: Option<SecretString>,
    bearer: Option<SecretString>,
    now: SystemTime,
    extra_headers: Option<Vec<(String, String)>>,
    pub(crate) source: Source,
    pub(crate) factory: Factory,
    supplied: Option<Client>,
}

impl<'probe> Harness<'probe> {
    pub(crate) fn new(probe: &'probe Probe) -> Self {
        Self {
            probe,
            params: probe.optional_params.clone(),
            images: probe.images.iter().cloned().map(EditImage::Text).collect(),
            api_key: probe
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.clone())),
            bearer: probe
                .configured_bearer_token
                .as_ref()
                .map(|token| SecretString::from(token.clone())),
            now: UNIX_EPOCH + Duration::from_secs(NOW_EPOCH_SECONDS),
            extra_headers: probe.extra_headers.as_ref().map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            }),
            source: Source::new(probe),
            factory: Factory::new(&probe.reply),
            supplied: (probe.supplied != Supplied::None).then(|| Client::new(&probe.reply)),
        }
    }

    pub(crate) fn call(&self) -> EditCall<'_> {
        EditCall {
            model: &self.probe.model,
            prompt: self.probe.prompt.as_deref(),
            images: &self.images,
            optional_params: self.params.clone(),
            api_base: self.probe.api_base.as_deref(),
            extra_headers: self.extra_headers.as_deref(),
            api_key: self.api_key.as_ref(),
            timeout: self.probe.timeout_ms.map(Duration::from_millis),
            aimage_edit: self.probe.aimage_edit,
            environment: EditEnvironment {
                region_name: self.probe.environment.region_name.as_deref(),
                region: self.probe.environment.region.as_deref(),
                session_region: self.probe.environment.session_region.as_deref(),
                runtime_endpoint: self.probe.environment.runtime_endpoint.as_deref(),
                bearer_token: self.bearer.as_ref(),
            },
            now: self.now,
        }
    }

    pub(crate) fn supplied(&self) -> Option<SuppliedClient<'_, Client>> {
        self.supplied.as_ref().map(|client| SuppliedClient {
            kind: match self.probe.supplied {
                Supplied::Async => ClientKind::Async,
                Supplied::None | Supplied::Sync => ClientKind::Sync,
            },
            transport: client,
        })
    }

    pub(crate) fn seen(&self) -> (Vec<ImageRequest>, Vec<ImageRequest>) {
        let supplied = self
            .supplied
            .as_ref()
            .map_or_else(Vec::new, |client| client.seen.requests());
        (supplied, self.factory.seen.requests())
    }
}
