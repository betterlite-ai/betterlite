#![cfg(test)]

use std::future::ready;
use std::task::{Context, Poll, Waker};

use secrecy::SecretString;

use super::{
    CustomParams, CustomSecretError, CustomSecretFuture, CustomSecretManager,
    CustomSecretManagerName, CustomSecretWrite,
};

struct ReadsOnly {
    name: String,
}

impl CustomSecretManager for ReadsOnly {
    fn secret_manager_name(&self) -> &str {
        &self.name
    }

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        _: &'borrowed str,
        _: Option<&'borrowed CustomParams>,
    ) -> CustomSecretFuture<'borrowed, Option<SecretString>> {
        Box::pin(ready(Ok(None)))
    }

    fn sync_read_secret(
        &self,
        _: &str,
        _: Option<&CustomParams>,
    ) -> Result<Option<SecretString>, CustomSecretError> {
        Ok(None)
    }
}

#[test]
fn custom_interface_behaviors() {
    manager_name_defaults_are_preserved();
    read_methods_are_required_and_callable();
    unsupported_writes_and_deletes_are_reported();
}

fn manager_name_defaults_are_preserved() {
    assert_eq!(
        CustomSecretManagerName::from(None).0,
        "custom_secret_manager"
    );
    assert_eq!(CustomSecretManagerName::from(Some("vault")).0, "vault");
}

fn read_methods_are_required_and_callable() {
    let manager = ReadsOnly {
        name: "custom_secret_manager".to_owned(),
    };
    assert_eq!(manager.secret_manager_name(), "custom_secret_manager");
    assert!(
        manager
            .sync_read_secret("missing", None)
            .expect("read should work")
            .is_none()
    );
    let mut context = Context::from_waker(Waker::noop());
    let mut read = manager.async_read_secret("missing", None);
    assert!(matches!(
        read.as_mut().poll(&mut context),
        Poll::Ready(Ok(None))
    ));
}

fn unsupported_writes_and_deletes_are_reported() {
    let manager = ReadsOnly {
        name: "custom_secret_manager".to_owned(),
    };
    let mut context = Context::from_waker(Waker::noop());
    let secret_value = SecretString::from("value");
    let mut write = manager.async_write_secret(CustomSecretWrite {
        secret_name: "name",
        secret_value: &secret_value,
        description: None,
        optional_params: None,
        tags: None,
    });
    let Poll::Ready(Err(write_error)) = write.as_mut().poll(&mut context) else {
        panic!("optional write should fail immediately");
    };
    assert!(matches!(
        write_error,
        CustomSecretError::NotImplemented { .. }
    ));
    let mut delete = manager.async_delete_secret("name", None, None);
    let Poll::Ready(Err(delete_error)) = delete.as_mut().poll(&mut context) else {
        panic!("optional delete should fail immediately");
    };
    assert!(matches!(
        delete_error,
        CustomSecretError::NotImplemented { .. }
    ));
}
