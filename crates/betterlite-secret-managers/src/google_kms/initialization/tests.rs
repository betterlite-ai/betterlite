#![cfg(test)]

use crate::google_kms::{DecryptFuture, DecryptTransport};

pub(super) struct Transport;

impl DecryptTransport for Transport {
    fn decrypt<'request>(
        &'request self,
        resource_name: &'request str,
        ciphertext: &'request [u8],
    ) -> DecryptFuture<'request> {
        Box::pin(async move {
            assert_eq!(resource_name, "resource");
            assert_eq!(ciphertext, b"ciphertext");
            Ok(b"plaintext".to_vec())
        })
    }
}
