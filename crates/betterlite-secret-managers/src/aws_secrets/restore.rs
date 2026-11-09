use std::error::Error;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, AwsTransport, CredentialResolver, NoBody,
};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::{SecretOptions, SecretTimeout};

use super::bodies::{CreateSecretBody, DescribeSecretReply, TagResourceBody, UpdateSecretBody};
use super::{SecretActionResponse, delete, post_reply};

const CREATE_SECRET: &str = "CreateSecret";
const DESCRIBE_SECRET: &str = "DescribeSecret";
const RESTORE_SECRET: &str = "RestoreSecret";
const UPDATE_SECRET: &str = "UpdateSecret";
const TAG_RESOURCE: &str = "TagResource";
const REDELETION_DAYS: u32 = 7;

pub(super) async fn create_or_restore<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    body: &CreateSecretBody<'_>,
    mut optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let request = ActionRequest {
        action: CREATE_SECRET,
        secret_name: body.name,
        request_data: Some(body),
    };
    let failure = match post_reply(manager, &request, optional_params.as_deref_mut(), timeout).await
    {
        Ok(response) => return Ok(response),
        Err(error) => error,
    };
    if !value_error(&failure) {
        return Err(failure);
    }
    let options = SecretOptions {
        optional_params: optional_params.as_deref(),
        timeout,
    };
    if !scheduled_for_deletion(manager, body.name, options).await? {
        return Err(failure);
    }
    let restore: ActionRequest<'_, NoBody> = ActionRequest {
        action: RESTORE_SECRET,
        secret_name: body.name,
        request_data: None,
    };
    post_reply(manager, &restore, optional_params.as_deref_mut(), timeout).await?;
    let update_body = UpdateSecretBody {
        secret_id: body.name,
        secret_string: body.secret_string,
        client_request_token: body.client_request_token,
        description: body.description,
        kms_key_id: body.kms_key_id,
    };
    update_and_tag(manager, body, &update_body, optional_params, timeout).await
}

async fn update_and_tag<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    body: &CreateSecretBody<'_>,
    update_body: &UpdateSecretBody<'_>,
    mut optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let update = ActionRequest {
        action: UPDATE_SECRET,
        secret_name: body.name,
        request_data: Some(update_body),
    };
    let updated = match post_reply(manager, &update, optional_params.as_deref_mut(), timeout).await
    {
        Ok(updated) => updated,
        Err(error) => {
            delete_after_failed_update(manager, body.name, &error, optional_params, timeout)
                .await?;
            return Err(error);
        }
    };
    let Some(tags) = body.tags else {
        return Ok(updated);
    };
    let tag_body = TagResourceBody {
        secret_id: body.name,
        tags,
    };
    let tag = ActionRequest {
        action: TAG_RESOURCE,
        secret_name: body.name,
        request_data: Some(&tag_body),
    };
    if let Err(error) = post_reply(manager, &tag, optional_params.as_deref_mut(), timeout).await {
        delete_after_failed_update(manager, body.name, &error, optional_params, timeout).await?;
        return Err(error);
    }
    Ok(updated)
}

// LiteLLM's except ValueError re-deletes the restored secret; every other failure propagates untouched
async fn delete_after_failed_update<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    error: &AwsReadError<Resolver::Error, Transport::Error>,
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<(), AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    if !value_error(error) {
        return Ok(());
    }
    delete::delete(
        manager,
        secret_name,
        REDELETION_DAYS,
        optional_params,
        timeout,
    )
    .await
    .map(drop)
}

async fn scheduled_for_deletion<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    options: SecretOptions<'_, AwsOptionalParams>,
) -> Result<bool, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let request: ActionRequest<'_, NoBody> = ActionRequest {
        action: DESCRIBE_SECRET,
        secret_name,
        request_data: None,
    };
    let mut optional_params = options.optional_params.map(super::copy_params);
    let failure = match manager
        .async_post_action::<NoBody, DescribeSecretReply>(
            &request,
            optional_params.as_mut(),
            options.timeout,
        )
        .await
    {
        Ok(reply) => return Ok(reply.deleted_date.is_some()),
        Err(error) => AwsReadError::Request(error),
    };
    if value_error(&failure) {
        return Ok(false);
    }
    Err(failure)
}

// LiteLLM's except ValueError catches an HTTP status error, an undecodable error body and a body that is not JSON
const fn value_error<ResolverError, TransportFailure>(
    error: &AwsReadError<ResolverError, TransportFailure>,
) -> bool
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    matches!(
        error,
        AwsReadError::Request(
            AwsRequestError::Http { .. }
                | AwsRequestError::ErrorText(_)
                | AwsRequestError::Response(_)
        )
    )
}
