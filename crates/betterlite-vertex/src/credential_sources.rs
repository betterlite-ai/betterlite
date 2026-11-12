pub mod adc;
pub mod aws;
pub mod error;
pub mod files;
pub mod getters;
pub mod handles;
pub mod sdk;
mod tests;

use secrecy::SecretString;

use error::CredentialSourceError;

// B8501 asks for one scope; the same application default chain the proxy uses supplies the token
pub async fn adc_access_token(
    scope: &str,
    deps: &adc::AdcDeps<'_>,
) -> Result<SecretString, CredentialSourceError> {
    let (handle, _project) = adc::credentials_from_default_auth(&[scope.to_owned()], deps).await?;
    handle.access_token().await
}
