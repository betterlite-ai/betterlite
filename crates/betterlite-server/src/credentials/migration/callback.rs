use betterlite_db::encryption_scan::{DbCipher, MigrationJson};
use betterlite_secrets::callback_vars::{Transform, transform_callback_vars};

// _migrate_callback_vars_table wraps decrypt_callback_vars(metadata) in encrypt_callback_vars over one complete mapping.
pub trait CallbackTransform {
    fn reencrypt(
        &self,
        metadata: &MigrationJson,
        cipher: &DbCipher,
    ) -> Result<MigrationJson, serde_json::Error>;
}

// B1600's transforms are infallible, so only the JSON bridge between them and the scanned value can fail.
pub struct CallbackVars;

impl CallbackTransform for CallbackVars {
    fn reencrypt(
        &self,
        metadata: &MigrationJson,
        cipher: &DbCipher,
    ) -> Result<MigrationJson, serde_json::Error> {
        let encoded = serde_json::to_value(metadata)?;
        let plain = transform_callback_vars(&encoded, &Transform::Decrypt(cipher));
        let sealed = transform_callback_vars(&plain, &Transform::Encrypt(cipher));
        serde_json::from_value(sealed)
    }
}
