use betterlite_bedrock::aws_settings::sources::{Credentials, SourceError};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_vertex::aws_federation::AwsCredentialsResolver;
use std::future::Future;
use std::future::ready;

// The server runs one Vertex workload identity pool source less than the SDK, so that source fails its load here.
pub(crate) struct UnavailableAws;

impl AwsCredentialsResolver for UnavailableAws {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Err(CredentialError::Source(SourceError::MissingProvider)))
    }
}
