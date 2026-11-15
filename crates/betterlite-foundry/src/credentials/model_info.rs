use secrecy::SecretString;

// the operator values come from litellm.api_base/api_key and the AZURE_AI_* variables; this slice holds only the precedence
#[derive(Debug, Clone, Copy)]
pub struct ModelInfoAccessors<'data> {
    pub global_api_base: Option<&'data str>,
    pub global_api_key: Option<&'data SecretString>,
    pub environment_api_base: Option<&'data str>,
    pub environment_api_key: Option<&'data SecretString>,
}

impl ModelInfoAccessors<'_> {
    #[must_use]
    pub fn api_base(self, explicit: Option<&str>) -> Option<String> {
        explicit
            .or(self.global_api_base)
            .or(self.environment_api_base)
            .map(str::to_owned)
    }

    #[must_use]
    pub fn api_key(self, explicit: Option<&SecretString>) -> Option<SecretString> {
        explicit
            .or(self.global_api_key)
            .or(self.environment_api_key)
            .cloned()
    }
}
