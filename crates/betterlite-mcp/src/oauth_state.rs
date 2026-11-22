mod test_support;

pub mod query;
pub mod redirect_uris;
pub mod sealed;
pub mod state;
pub mod state_cookie;
pub mod token_rules;

pub trait SealCipher {
    fn encrypt(&self, plain: &str) -> String;
    fn decrypt(&self, value: &str, field: &str) -> Option<String>;
}
