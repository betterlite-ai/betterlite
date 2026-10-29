#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "serde_json test fixtures are concise and type-directed"
    )
)]

pub mod callback;
pub mod callback_vars;
pub mod error;
pub mod master_key;
pub mod secret_codec;
pub mod stored_cipher;
