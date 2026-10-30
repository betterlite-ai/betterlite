#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Mutex;

use thiserror::Error;

use crate::sources::parameters::Parameters;
use crate::sources::storage::{ContentParameters, EvidenceParameters, SampleParameters, Storage};

#[derive(Debug, Error)]
#[error("stub storage failure")]
pub(crate) struct StubError;

pub(crate) struct StubStorage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) seen_sample: Mutex<Option<SampleParameters>>,
    pub(crate) seen_content: Mutex<Option<ContentParameters>>,
    pub(crate) seen_evidence: Mutex<Option<EvidenceParameters>>,
}

impl StubStorage {
    pub(crate) fn with_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            seen_sample: Mutex::new(None),
            seen_content: Mutex::new(None),
            seen_evidence: Mutex::new(None),
        }
    }
}

impl Storage for StubStorage {
    type Error = StubError;

    fn lens_availability(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(self.bytes.clone()))
    }

    fn lens_agents(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(self.bytes.clone()))
    }

    fn lens_sample(
        &self,
        parameters: SampleParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let recorded = self
            .seen_sample
            .lock()
            .map(|mut seen| *seen = Some(parameters));
        ready(
            recorded
                .map(|()| self.bytes.clone())
                .map_err(|_poison| StubError),
        )
    }

    fn lens_content(
        &self,
        parameters: ContentParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let recorded = self
            .seen_content
            .lock()
            .map(|mut seen| *seen = Some(parameters));
        ready(
            recorded
                .map(|()| self.bytes.clone())
                .map_err(|_poison| StubError),
        )
    }

    fn lens_evidence(
        &self,
        parameters: EvidenceParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let recorded = self
            .seen_evidence
            .lock()
            .map(|mut seen| *seen = Some(parameters));
        ready(
            recorded
                .map(|()| self.bytes.clone())
                .map_err(|_poison| StubError),
        )
    }
}
