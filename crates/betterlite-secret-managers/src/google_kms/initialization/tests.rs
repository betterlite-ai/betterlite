#![cfg(test)]

use std::path::Path;

use crate::google_kms::{DecryptFuture, DecryptTransport};

pub(super) struct Transport;

impl DecryptTransport for Transport {
    fn decrypt<'request>(
        &'request self,
        credentials_path: &'request Path,
        resource_name: &'request str,
        ciphertext: &'request [u8],
    ) -> DecryptFuture<'request> {
        Box::pin(async move {
            assert_eq!(credentials_path, Path::new("credentials.json"));
            assert_eq!(resource_name, "resource");
            assert_eq!(ciphertext, b"ciphertext");
            Ok(b"plaintext".to_vec())
        })
    }
}
