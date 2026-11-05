use std::error::Error;
use std::time::SystemTime;

use secrecy::{ExposeSecret, SecretSlice, SecretString};
use serde::Serialize;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::sigv4::signed_headers;
use crate::aws::request_signing::{
    ActionRequest, AwsCredentialsInfo, AwsOptionalParams, AwsRequestError, CredentialResolver,
    PreparedRequest,
};

const RUNTIME_ENDPOINT_VARIABLE: &str = "AWS_BEDROCK_RUNTIME_ENDPOINT";

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct DefaultBody<'borrowed> {
    secret_id: &'borrowed str,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret_string: Option<&'borrowed str>,
}

impl<Resolver> AwsSecretsManager<Resolver>
where
    Resolver: CredentialResolver,
{
    pub fn prepare_request(
        &self,
        request: &ActionRequest<'_>,
        secret_value: Option<&SecretString>,
        optional_params: Option<&mut AwsOptionalParams>,
        now: SystemTime,
    ) -> Result<PreparedRequest, AwsRequestError<Resolver::Error>> {
        let mut fresh_params = AwsOptionalParams::default();
        let params = optional_params.unwrap_or(&mut fresh_params);
        self.fill_missing_optional_params(params);
        let credentials_info = self
            .dependencies
            .resolver
            .resolve_credentials(params)
            .map_err(AwsRequestError::Credentials)?;
        let endpoint_url = self.endpoint_url(&credentials_info);
        let body = request_body(request, secret_value)?;
        let headers = signed_headers(&credentials_info, &endpoint_url, request.action, &body, now)?;
        Ok(PreparedRequest {
            endpoint_url,
            headers,
            body: SecretSlice::from(body),
        })
    }

    fn endpoint_url(&self, credentials_info: &AwsCredentialsInfo) -> String {
        let explicit_endpoint = credentials_info
            .aws_bedrock_runtime_endpoint
            .as_ref()
            .filter(|endpoint| !endpoint.is_empty())
            .or_else(|| {
                self.dependencies
                    .environment
                    .get(RUNTIME_ENDPOINT_VARIABLE)
                    .filter(|endpoint| !endpoint.is_empty())
            });
        explicit_endpoint.map_or_else(
            || {
                let region = &credentials_info.aws_region_name;
                let suffix = (self.dependencies.dns_suffix)(region);
                format!("https://secretsmanager.{region}.{suffix}")
            },
            |endpoint| endpoint.replace("bedrock-runtime", "secretsmanager"),
        )
    }
}

fn request_body<ResolverError>(
    request: &ActionRequest<'_>,
    secret_value: Option<&SecretString>,
) -> Result<Vec<u8>, AwsRequestError<ResolverError>>
where
    ResolverError: Error + 'static,
{
    if let Some(data) = request.request_data.filter(|data| !data.is_empty()) {
        return serde_json::to_vec(data).map_err(AwsRequestError::Body);
    }
    let secret_string = secret_value
        .map(ExposeSecret::expose_secret)
        .filter(|value| request.action == "PutSecretValue" && !value.is_empty());
    serde_json::to_vec(&DefaultBody {
        secret_id: request.secret_name,
        secret_string,
    })
    .map_err(AwsRequestError::Body)
}
