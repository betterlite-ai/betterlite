use secrecy::SecretString;

#[derive(Debug, Clone, Default)]
pub struct NativeSpeechIdentity {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}
