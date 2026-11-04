#![cfg(test)]

use std::future;
use std::sync::Mutex;
use std::sync::PoisonError;

use super::super::{
    BaseSecretManager, SecretContractError, SecretFuture, SecretOperationOptions, SecretResponse,
    SecretValue, SecretWrite,
};

type OptionalSecretFuture<'borrowed> = SecretFuture<'borrowed, Option<String>>;
type DeleteSecretFuture<'borrowed> = SecretFuture<'borrowed, SecretResponse>;
type ReadOptions<'borrowed> = SecretOperationOptions<'borrowed>;
type DeleteOptions<'borrowed> = SecretOperationOptions<'borrowed>;

#[derive(Default)]
pub(super) struct FakeManager {
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) missing: Mutex<Vec<String>>,
}

impl FakeManager {
    fn read_async<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
    ) -> OptionalSecretFuture<'borrowed> {
        Box::pin(async move {
            self.read(secret_name)
                .map(|secret| secret.map(|value| format!("read:{value}")))
        })
    }

    fn read(&self, name: &str) -> Result<Option<String>, SecretContractError> {
        self.record(format!("read:{name}"))?;
        let missing = self.missing.lock().map_err(|error| lock_error(&error))?;
        Ok((!missing.iter().any(|entry| entry == name)).then(|| String::from("value")))
    }

    fn write(&self, name: &str) -> Result<SecretResponse, SecretContractError> {
        self.record(format!("write:{name}"))?;
        let entry = (String::from("name"), SecretValue::String(name.to_owned()));
        Ok(SecretResponse::from([entry]))
    }

    fn delete(
        &self,
        name: &str,
        recovery_days: u32,
    ) -> Result<SecretResponse, SecretContractError> {
        self.record(format!("delete:{name}:{recovery_days}"))?;
        Ok(SecretResponse::new())
    }

    fn record(&self, call: String) -> Result<(), SecretContractError> {
        self.calls
            .lock()
            .map_err(|error| lock_error(&error))?
            .push(call);
        Ok(())
    }
}

impl BaseSecretManager for FakeManager {
    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        _options: ReadOptions<'borrowed>,
    ) -> OptionalSecretFuture<'borrowed> {
        self.read_async(secret_name)
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        _options: SecretOperationOptions<'_>,
    ) -> Result<Option<String>, SecretContractError> {
        self.record(format!("sync_read:{secret_name}"))?;
        Ok(self
            .read(secret_name)?
            .or_else(|| Some(String::from("sync value"))))
    }

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed>,
    ) -> SecretFuture<'borrowed, SecretResponse> {
        let response = self.write(write.secret_name);
        Box::pin(future::ready(response))
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        recovery_window_in_days: Option<u32>,
        _options: DeleteOptions<'borrowed>,
    ) -> DeleteSecretFuture<'borrowed> {
        let result = recovery_window_in_days.map_or_else(
            || self.delete(secret_name, 7),
            |days| self.delete(secret_name, days),
        );
        Box::pin(async move { result })
    }
}

fn lock_error<T>(error: &PoisonError<T>) -> SecretContractError {
    SecretContractError::Operation {
        message: error.to_string(),
    }
}
