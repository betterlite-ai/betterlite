use secrecy::{ExposeSecret as _, SecretString};

#[path = "resource_sdk_settings/tests.rs"]
mod tests;

#[path = "resource_sdk_settings/timeout.rs"]
pub mod timeout;

#[derive(Clone, Debug)]
pub struct ResourceSdkSettings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub openai_organization: Option<String>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub vertexai_project: Option<String>,
    pub vertexai_location: Option<String>,
    pub vertexai_credentials: Option<SecretString>,
}

#[derive(Debug, Default)]
pub struct ResourceSdkRequestOptions {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
}

macro_rules! fallback_accessors {
    ($value:ty, $borrow:ident, $empty:expr; $(
        $method:ident: $request_field:ident => [$($setting:ident),*];
    )+) => {$(
        #[must_use]
        pub fn $method<'settings>(
            &'settings self,
            request: &'settings ResourceSdkRequestOptions,
        ) -> Option<&'settings $value> {
            let candidates = [request.$request_field.$borrow(), $(self.$setting.$borrow()),*];
            candidates.into_iter().flatten().find(|value| !($empty)(value))
        }
    )+};
}

impl ResourceSdkSettings {
    fallback_accessors! {
        str, as_deref, str::is_empty;
        azure_base: api_base => [api_base, azure_api_base];
        azure_version: api_version => [api_version, azure_api_version];
        openai_organization: organization => [organization, openai_organization];
        openai_api_version: api_version => [];
        azure_organization: organization => [];
        vertex_project: vertex_project => [vertex_project, vertexai_project];
        vertex_location: vertex_location => [vertex_location, vertexai_location];
    }

    fallback_accessors! {
        SecretString, as_ref, |value: &SecretString| value.expose_secret().is_empty();
        openai_api_key: api_key => [api_key, openai_key, openai_api_key];
        azure_api_key: api_key => [api_key, azure_key, azure_openai_api_key, azure_api_key];
        vertex_credentials: vertex_credentials => [vertexai_credentials];
    }

    #[must_use]
    pub fn openai_base(&self, request: &ResourceSdkRequestOptions) -> String {
        let candidates = [
            request.api_base.as_deref(),
            self.api_base.as_deref(),
            self.openai_base_url.as_deref(),
            self.openai_api_base.as_deref(),
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|value| !value.is_empty())
            .unwrap_or("https://api.openai.com/v1")
            .to_owned()
    }
}
