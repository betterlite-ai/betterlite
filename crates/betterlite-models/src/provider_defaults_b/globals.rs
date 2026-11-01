use secrecy::SecretString;

#[derive(Debug, Clone, Default)]
pub struct GlobalCredentials {
    // Mirrors litellm's module-level SDK settings, resolved by the caller from litellm_settings, never read from an OS environment variable here.
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub xai_key: Option<String>,
    pub inception_key: Option<String>,
    pub lemonade_key: Option<String>,
}
