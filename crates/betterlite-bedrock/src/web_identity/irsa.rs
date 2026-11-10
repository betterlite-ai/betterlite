use super::error::WebIdentityError;
use super::sts::{identity, required_credentials, sts_client, sts_response};
use crate::assume_role::expiry::StsResponse;
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::sources::NativeSources;
use crate::aws_settings::{AwsSessionTag, assume_role_parameters};
use aws_sdk_sts::Client;
use aws_sdk_sts::types::Credentials as ResponseCredentials;
use aws_sdk_sts::types::Tag;
use aws_smithy_types::error::display::DisplayErrorContext;
use secrecy::{ExposeSecret as _, SecretString};
use std::path::Path;
pub struct SameAccountIrsa<'text> {
    pub role_arn: &'text str,
    pub session_name: &'text str,
    pub external_id: Option<&'text str>,
    pub tags: Option<&'text [AwsSessionTag]>,
}
pub struct CrossAccountIrsa<'text> {
    pub irsa_role_arn: &'text str,
    pub role_arn: &'text str,
    pub session_name: &'text str,
    pub token_file: &'text str,
    pub external_id: Option<&'text str>,
    pub tags: Option<&'text [AwsSessionTag]>,
}
pub async fn handle_irsa_same_account(
    input: &SameAccountIrsa<'_>,
    sts: &StsClientKwargs,
    native: &NativeSources,
) -> Result<StsResponse, WebIdentityError> {
    tracing::debug!("Same account role assumption, using automatic IRSA");
    let client = sts_client(sts, native, None).await;
    log_caller_identity(&client, "Current IRSA identity: ").await;
    tracing::debug!(
        "Attempting to assume role: {} with session: {}",
        input.role_arn,
        input.session_name
    );
    send_assume_role(&client, input).await
}
pub async fn handle_irsa_cross_account(
    input: &CrossAccountIrsa<'_>,
    sts: &StsClientKwargs,
    native: &NativeSources,
) -> Result<StsResponse, WebIdentityError> {
    tracing::debug!("Cross-account role assumption detected");
    let web_identity_token = read_token_file(input.token_file, native).await?;
    let irsa_credentials = assume_irsa_role(sts, native, input, web_identity_token).await?;
    let target_client = sts_client(sts, native, Some(identity(&irsa_credentials))).await;
    log_caller_identity(
        &target_client,
        "Current identity after manual IRSA assumption: ",
    )
    .await;
    tracing::debug!(
        "Attempting to assume target role: {} with session: {}",
        input.role_arn,
        input.session_name
    );
    let target = SameAccountIrsa {
        role_arn: input.role_arn,
        session_name: input.session_name,
        external_id: input.external_id,
        tags: input.tags,
    };
    send_assume_role(&target_client, &target).await
}
async fn assume_irsa_role(
    sts: &StsClientKwargs,
    native: &NativeSources,
    input: &CrossAccountIrsa<'_>,
    web_identity_token: SecretString,
) -> Result<ResponseCredentials, WebIdentityError> {
    let irsa_client = sts_client(sts, native, None).await;
    tracing::debug!(
        "Manually assuming IRSA role {} with session {}",
        input.irsa_role_arn,
        input.session_name
    );
    let response = irsa_client
        .assume_role_with_web_identity()
        .role_arn(input.irsa_role_arn)
        .role_session_name(input.session_name)
        .web_identity_token(web_identity_token.expose_secret())
        .send()
        .await
        .map_err(|source| WebIdentityError::WebIdentity {
            source: Box::new(source),
        })?;
    required_credentials(response.credentials(), "web identity").cloned()
}
async fn read_token_file(
    token_file: &str,
    native: &NativeSources,
) -> Result<SecretString, WebIdentityError> {
    let path = Path::new(token_file);
    let bytes = native
        .filesystem
        .read_to_end(path)
        .await
        .map_err(|source| WebIdentityError::TokenFile {
            path: path.to_owned(),
            source,
        })?;
    let text = String::from_utf8(bytes).map_err(|source| WebIdentityError::TokenFileEncoding {
        path: path.to_owned(),
        source,
    })?;
    Ok(SecretString::from(text.trim().to_owned()))
}
async fn log_caller_identity(client: &Client, prefix: &str) {
    match client.get_caller_identity().send().await {
        Ok(identity) => tracing::debug!("{prefix}{}", identity.arn().unwrap_or("unknown")),
        Err(error) => tracing::debug!(
            "Failed to get caller identity: {}",
            DisplayErrorContext(&error)
        ),
    }
}
async fn send_assume_role(
    client: &Client,
    input: &SameAccountIrsa<'_>,
) -> Result<StsResponse, WebIdentityError> {
    let params = assume_role_parameters(
        input.role_arn.to_owned(),
        input.session_name.to_owned(),
        input.external_id.map(str::to_owned),
        input.tags.map(<[AwsSessionTag]>::to_vec),
    );
    let tags = params.tags.as_deref().map(tags_to_sdk).transpose()?;
    let response = client
        .assume_role()
        .role_arn(params.role_arn)
        .role_session_name(params.role_session_name)
        .set_external_id(params.external_id)
        .set_tags(tags)
        .send()
        .await
        .map_err(|source| WebIdentityError::AssumeRole {
            source: Box::new(source),
        })?;
    sts_response(required_credentials(response.credentials(), "assume role")?)
}
fn tags_to_sdk(tags: &[AwsSessionTag]) -> Result<Vec<Tag>, WebIdentityError> {
    tags.iter()
        .map(|tag| {
            Tag::builder()
                .key(tag.key.clone())
                .value(tag.value.clone())
                .build()
                .map_err(|source| WebIdentityError::Tag { source })
        })
        .collect()
}
