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

impl ResourceSdkSettings {
    #[must_use]
    pub fn openai_base(&self, request: &ResourceSdkRequestOptions) -> String {
        let value = first_truthy([
            request.api_base.as_deref(),
            self.api_base.as_deref(),
            self.openai_base_url.as_deref(),
            self.openai_api_base.as_deref(),
            Some("https://api.openai.com/v1"),
        ]);
        value.to_owned()
    }

    #[must_use]
    pub fn openai_api_key<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings SecretString> {
        if let Some(value) = first_secret([
            request.api_key.as_ref(),
            self.api_key.as_ref(),
            self.openai_key.as_ref(),
            self.openai_api_key.as_ref(),
        ]) {
            return Some(value);
        }
        None
    }

    #[must_use]
    pub fn azure_api_key<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings SecretString> {
        let candidates = [
            request.api_key.as_ref(),
            self.api_key.as_ref(),
            self.azure_key.as_ref(),
            self.azure_openai_api_key.as_ref(),
            self.azure_api_key.as_ref(),
        ];
        first_secret(candidates)
    }

    #[must_use]
    pub fn azure_base<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.api_base.as_deref(),
            self.api_base.as_deref(),
            self.azure_api_base.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn azure_version<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.api_version.as_deref(),
            self.api_version.as_deref(),
            self.azure_api_version.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn openai_organization<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.organization.as_deref(),
            self.organization.as_deref(),
            self.openai_organization.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn openai_api_version<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        if let Some(value) = request
            .api_version
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            return Some(value);
        }
        None
    }

    #[must_use]
    pub fn azure_organization<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        if let Some(value) = request
            .organization
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            return Some(value);
        }
        None
    }

    #[must_use]
    pub fn vertex_base<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> &'settings str {
        if let Some(value) = request.api_base.as_deref() {
            return value;
        }
        ""
    }

    #[must_use]
    pub fn vertex_project<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.vertex_project.as_deref(),
            self.vertex_project.as_deref(),
            self.vertexai_project.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn vertex_location<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.vertex_location.as_deref(),
            self.vertex_location.as_deref(),
            self.vertexai_location.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn vertex_credentials<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings SecretString> {
        if let Some(value) = first_secret([
            request.vertex_credentials.as_ref(),
            self.vertexai_credentials.as_ref(),
        ]) {
            return Some(value);
        }
        None
    }
}

fn first_truthy<const N: usize>(values: [Option<&str>; N]) -> &str {
    let candidate = values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or("");
    if candidate.is_empty() { "" } else { candidate }
}

fn first_secret<const N: usize>(values: [Option<&SecretString>; N]) -> Option<&SecretString> {
    if let Some(value) = values
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
    {
        return Some(value);
    }
    None
}
