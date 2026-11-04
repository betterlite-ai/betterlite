#![cfg(test)]

pub(super) struct Transport;

impl crate::google_kms::DecryptTransport for Transport {
    fn decrypt<'a>(
        &'a self,
        resource_name: &'a str,
        ciphertext: &'a [u8],
    ) -> crate::google_kms::DecryptFuture<'a> {
        Box::pin(async move {
            assert_eq!(resource_name, "resource");
            assert_eq!(ciphertext, b"ciphertext");
            Ok(b"plaintext".to_vec())
        })
    }
}
