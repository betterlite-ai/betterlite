#![cfg(test)]

use std::future;
use std::sync::{Mutex, PoisonError};

use secrecy::SecretString;

use super::super::options::{DeleteOptions, SecretOptions, SecretTags, SecretWrite};
use super::super::{BaseSecretManager, SecretFuture};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Params {
    pub(super) version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum TestError {
    #[error("request timed out")]
    Timeout,
    #[error("HTTP status failure")]
    HttpStatus,
}

#[derive(Debug)]
pub(super) enum Operation {
    Read,
    Write {
        value: SecretString,
        description: Option<String>,
        tags: Option<SecretTags>,
    },
    Delete(u32),
}

#[derive(Debug)]
pub(super) struct Call {
    pub(super) name: String,
    pub(super) operation: Operation,
    pub(super) params: Option<Params>,
    pub(super) timeout: Option<super::super::options::SecretTimeout>,
}

#[derive(Default)]
pub(super) struct FakeManager {
    pub(super) calls: Mutex<Vec<Call>>,
    pub(super) missing: Option<&'static str>,
    pub(super) fail_at: Option<(usize, TestError)>,
}

impl FakeManager {
    fn record(
        &self,
        name: &str,
        operation: Operation,
        options: SecretOptions<'_, Params>,
    ) -> Result<(), TestError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(Call {
            name: name.to_owned(),
            operation,
            params: options.optional_params.copied(),
            timeout: options.timeout,
        });
        let count = calls.len();
        drop(calls);
        if let Some((step, error)) = &self.fail_at
            && count == *step
        {
            return Err(error.clone());
        }
        Ok(())
    }
}

impl BaseSecretManager for FakeManager {
    type Params = Params;
    type Response = &'static str;
    type Error = TestError;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, Params>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, TestError> {
        Box::pin(async move { self.sync_read_secret(secret_name, options) })
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOptions<'_, Params>,
    ) -> Result<Option<SecretString>, TestError> {
        self.record(secret_name, Operation::Read, options)?;
        Ok((self.missing != Some(secret_name)).then(|| SecretString::from("")))
    }

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed, Params>,
    ) -> SecretFuture<'borrowed, &'static str, TestError> {
        Box::pin(async move {
            self.record(
                write.secret_name,
                Operation::Write {
                    value: write.secret_value.clone(),
                    description: write.description.map(str::to_owned),
                    tags: write.tags.cloned(),
                },
                write.options,
            )?;
            Ok("created")
        })
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, Params>,
    ) -> SecretFuture<'borrowed, &'static str, TestError> {
        let deleted = self
            .record(
                secret_name,
                Operation::Delete(options.recovery_window_in_days),
                options.operation,
            )
            .map(|()| "deleted");
        Box::pin(future::ready(deleted))
    }
}
