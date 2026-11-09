use secrecy::SecretString;

#[derive(Clone, Debug, Default)]
pub struct CliContext {
    pub base_url: String,
    pub base_url_explicit: bool,
    pub api_key: Option<SecretString>,
    pub api_key_from_token_file: bool,
}

mod tests;
