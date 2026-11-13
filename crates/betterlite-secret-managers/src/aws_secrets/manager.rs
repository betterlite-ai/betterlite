use secrecy::SecretString;

use crate::aws::request_signing::{AwsOptionalParams, CredentialResolver};
use crate::aws_secret_reads::AwsReadError;
use crate::aws_secret_reads::blocking::AwsBlockingTransport;
use crate::secret_contracts::options::{DeleteOptions, SecretOptions, SecretRotation, SecretWrite};
use crate::secret_contracts::{BaseSecretManager, RotationError, SecretFuture};

use super::SecretActionResponse;
use super::{AwsSecretClient, copy_params, delete, rotate};

impl<Resolver, Transport> BaseSecretManager for AwsSecretClient<'_, Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsBlockingTransport,
{
    type Params = AwsOptionalParams;
    type Response = SecretActionResponse;
    type Error = AwsReadError<Resolver::Error, Transport::Error>;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Self::Error> {
        let manager = self.manager;
        Box::pin(async move {
            let mut params = options.optional_params.map(copy_params);
            manager
                .async_read_secret(secret_name, params.as_mut(), options.timeout, None)
                .await
        })
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOptions<'_, Self::Params>,
    ) -> Result<Option<SecretString>, Self::Error> {
        let manager = self.manager;
        let mut params = options.optional_params.map(copy_params);
        manager.sync_read_secret(secret_name, params.as_mut(), options.timeout, None)
    }

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error> {
        Box::pin(super::write::write(self.manager, write, self.next_token))
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error> {
        Box::pin(delete::delete_options(self.manager, secret_name, options))
    }

    fn async_rotate_secret<'borrowed>(
        &'borrowed self,
        rotation: SecretRotation<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, RotationError<Self::Error>> {
        Box::pin(rotate::rotate(self.manager, rotation, self.next_token))
    }
}
