use std::borrow::Cow;
use std::iter::once;
use std::time::Duration;

use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SignatureLocation, SigningParams, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use percent_encoding::utf8_percent_encode;
use secrecy::SecretString;

use super::error::RdsCredentialsError;
use super::selection;
use super::{
    RDS_ESCAPE, RDS_SERVICE_NAME, RDS_TOKEN_LIFETIME_SECONDS, RdsCredentialInputs, RdsEndpoint,
    RdsSession, RdsSigner, RdsSources, rds_timeout_config,
};

type SignedParameter = (&'static str, Cow<'static, str>);

impl RdsSigner {
    pub async fn init(
        inputs: RdsCredentialInputs,
        sources: RdsSources<'_>,
    ) -> Result<Self, RdsCredentialsError> {
        let selection = selection::prepare(inputs, sources.secrets)?;
        let client_timeout = rds_timeout_config(selection.timeout)?;
        let credentials = selection::select(&selection, &sources).await?;
        Ok(Self {
            client_timeout,
            session: RdsSession {
                credentials,
                region: selection.region,
                clock: sources.clock.clone(),
            },
        })
    }

    pub fn db_auth_token(
        &self,
        endpoint: &RdsEndpoint<'_>,
    ) -> Result<SecretString, RdsCredentialsError> {
        let operation = format!(
            "Action=connect&DBUser={}",
            utf8_percent_encode(endpoint.user, RDS_ESCAPE)
        );
        let uri = format!("https://{}:{}/?{operation}", endpoint.host, endpoint.port);
        // botocore signs urlsplit().hostname, which is lowercase, and leaves out the https default port
        let host = signed_host(endpoint.host, endpoint.port);
        let identity = Identity::from(self.session.credentials.clone());
        let mut settings = SigningSettings::default();
        settings.signature_location = SignatureLocation::QueryParams;
        settings.expires_in = Some(Duration::from_secs(RDS_TOKEN_LIFETIME_SECONDS));
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.session.region)
            .name(RDS_SERVICE_NAME)
            .time(self.session.clock.now())
            .settings(settings)
            .build()
            .map_err(|source| RdsCredentialsError::SigningParams { source })?;
        let headers = once(("host", host.as_str()));
        let signable = SignableRequest::new("GET", uri.as_str(), headers, SignableBody::Bytes(&[]))
            .map_err(|source| RdsCredentialsError::SignableRequest { source })?;
        let output = sign(signable, &SigningParams::from(params))
            .map_err(|source| RdsCredentialsError::Sign { source })?;
        let (instructions, signature) = output.into_parts();
        let (_, parameters) = instructions.into_parts();
        let query = token_query(operation, parameters, &signature);
        Ok(SecretString::from(format!(
            "{}:{}/?{query}",
            endpoint.host, endpoint.port
        )))
    }
}

fn signed_host(host: &str, port: &str) -> String {
    let name = host.to_ascii_lowercase();
    match port.parse::<u16>() {
        Ok(443) | Err(_) => name,
        Ok(number) => format!("{name}:{number}"),
    }
}

// botocore appends the security token after the signed headers and the signature last
fn token_query(operation: String, parameters: Vec<SignedParameter>, signature: &str) -> String {
    let mut query = operation;
    for (name, value) in parameters {
        if name == "X-Amz-Signature" {
            continue;
        }
        query.push('&');
        query.push_str(name);
        query.push('=');
        query.push_str(&utf8_percent_encode(value.as_ref(), RDS_ESCAPE).to_string());
    }
    query.push_str("&X-Amz-Signature=");
    query.push_str(signature);
    query
}
