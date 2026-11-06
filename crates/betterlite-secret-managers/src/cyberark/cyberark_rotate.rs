use std::convert::Infallible;

use super::CyberArkSecretManager;
use super::cyberark_delete::delete;
use super::cyberark_write::write;
use super::operation::SecretOperation;
use super::transport::ConjurTransport;
use crate::secret_contracts::RotationError;
use crate::secret_contracts::options::SecretRotation;

// The shared rotation rejects equal names; LiteLLM's base algorithm accepts them and still deletes.
pub(super) async fn rotate<T>(
    manager: &CyberArkSecretManager<T>,
    rotation: SecretRotation<'_, ()>,
) -> Result<SecretOperation, RotationError<Infallible>>
where
    T: ConjurTransport,
{
    manager
        .read(rotation.current_secret_name)
        .await
        .ok_or(RotationError::CurrentSecretNotFound)?;
    let response = write(manager, rotation.new_secret_name, rotation.new_secret_value).await;
    manager
        .read(rotation.new_secret_name)
        .await
        .ok_or(RotationError::NewSecretNotVerified)?;
    delete(manager, rotation.current_secret_name);
    Ok(response)
}
