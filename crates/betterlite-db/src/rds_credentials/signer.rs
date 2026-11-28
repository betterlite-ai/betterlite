use std::time::Duration;

use betterlite_sigv4::credentials::Credentials as SigningCredentials;
use betterlite_sigv4::sign::{Signable, SigningMode, presign_request};
use percent_encoding::utf8_percent_encode;
use secrecy::SecretString;

use super::error::RdsCredentialsError;
use super::selection;
use super::{
    RDS_ESCAPE, RDS_SERVICE_NAME, RDS_TOKEN_LIFETIME_SECONDS, RdsCredentialInputs, RdsEndpoint,
    RdsSession, RdsSigner, RdsSources, rds_timeout_config,
};

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
        let headers = [("host".to_owned(), signed_host(endpoint.host, endpoint.port))];
        let signing = SigningCredentials::from(&self.session.credentials);
        let parameters = presign_request(
            &Signable {
                method: "GET",
                url: &uri,
                headers: &headers,
                body: &[],
                service: RDS_SERVICE_NAME,
                region: &self.session.region,
                credentials: &signing,
                mode: SigningMode::SigV4Auth,
                now: self.session.clock.now(),
            },
            Duration::from_secs(RDS_TOKEN_LIFETIME_SECONDS),
        )
        .map_err(|source| RdsCredentialsError::Sign { source })?;
        let query = token_query(operation, parameters);
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

fn token_query(operation: String, parameters: Vec<(String, String)>) -> String {
    let mut query = operation;
    for (name, value) in parameters {
        query.push('&');
        query.push_str(&name);
        query.push('=');
        query.push_str(&utf8_percent_encode(&value, RDS_ESCAPE).to_string());
    }
    query
}
