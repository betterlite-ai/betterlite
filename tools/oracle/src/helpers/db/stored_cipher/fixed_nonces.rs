use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::NonceSource;

pub(super) struct FixedNonces(pub(super) Vec<u8>);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}
