use std::convert::Infallible;
use std::future::Future;

use betterlite_lens::sources::parameters::Parameters;
use betterlite_lens::sources::storage::{
    ContentParameters, EvidenceParameters, SampleParameters, Storage,
};

pub(crate) struct FakeStorage(pub(crate) Vec<u8>);

impl Storage for FakeStorage {
    type Error = Infallible;

    fn lens_availability(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let bytes = self.0.clone();
        async move { Ok(bytes) }
    }

    fn lens_agents(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let bytes = self.0.clone();
        async move { Ok(bytes) }
    }

    fn lens_sample(
        &self,
        _parameters: SampleParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let bytes = self.0.clone();
        async move { Ok(bytes) }
    }

    fn lens_content(
        &self,
        _parameters: ContentParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let bytes = self.0.clone();
        async move { Ok(bytes) }
    }

    fn lens_evidence(
        &self,
        _parameters: EvidenceParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let bytes = self.0.clone();
        async move { Ok(bytes) }
    }
}
