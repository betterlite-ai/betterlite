use std::collections::BTreeMap;

use betterlite_azure::resource_urls::error::ResourceUrlError;
use betterlite_azure::resource_urls::{AzureResource, ResourceAuth, UrlRequest};
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_openai::videos::create::VideoCreate;
use secrecy::SecretString;

use super::fixture::{NoSources, credentials, leaf_error, parse};
use crate::azure_resource_dispatch::resource_urls::{self, Call, Outcome};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};

struct Fixtures {
    credentials: DeploymentSettings,
    sources: NoSources,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            credentials: credentials(),
            sources: NoSources,
        }
    }

    fn auth<'auth>(
        &'auth self,
        fallback_api_key: Option<&'auth SecretString>,
    ) -> ResourceAuth<'auth> {
        ResourceAuth {
            headers: BTreeMap::new(),
            fallback_api_key,
            credentials: &self.credentials,
            refresh_enabled: false,
            provider: None,
            sources: &self.sources,
        }
    }
}

fn outcome(call: Call<'_>) -> Result<Outcome, ResourceUrlError> {
    let result = super::block_on(dispatch(AzureResourceOperation::ResourceUrls(Box::new(
        resource_urls::Dispatch { call },
    ))));
    match result {
        Ok(AzureResourceResult::ResourceUrls(outcome)) => Ok(outcome),
        Err(AzureResourceError::ResourceUrls(error)) => Err(error),
        Ok(_) | Err(_) => panic!("the resource urls dispatch returns its own outcome"),
    }
}

fn resource_url(
    api_base: Option<&str>,
    api_version: Option<&str>,
    resource: AzureResource,
) -> Result<Outcome, ResourceUrlError> {
    outcome(Call::Url {
        resource,
        request: UrlRequest {
            api_base,
            fallback_api_base: None,
            api_version,
        },
    })
}

fn container_url_rows() {
    let cases = [
        (
            Some("https://res.test"),
            Some("2024-10-21"),
            "https://res.test/openai/containers?api-version=2024-10-21",
        ),
        (
            Some("https://res.test/openai/responses?api-version=2024-10-21"),
            None,
            "https://res.test/openai/containers?api-version=2024-10-21",
        ),
    ];
    for (api_base, api_version, expected) in cases {
        let Outcome::Url(url) = resource_url(api_base, api_version, AzureResource::Container)
            .expect("the container url builds")
        else {
            panic!("the url call returns its endpoint");
        };
        assert_eq!(url, expected);
    }
}

fn video_vector_url_rows() {
    let Outcome::Url(video) = resource_url(Some("https://res.test"), None, AzureResource::Video)
        .expect("the video url builds")
    else {
        panic!("the url call returns its endpoint");
    };
    assert_eq!(video, "https://res.test/openai/v1/videos");
    let Outcome::Url(vector) = resource_url(
        Some("https://res.test"),
        Some("2024-10-21"),
        AzureResource::VectorStore,
    )
    .expect("the vector store url builds") else {
        panic!("the url call returns its endpoint");
    };
    assert_eq!(
        vector,
        "https://res.test/openai/vector_stores?api-version=2024-10-21"
    );
}

fn environment_headers_row() {
    let fixtures = Fixtures::new();
    let key = SecretString::from("key-1");
    let Outcome::Headers(headers) = outcome(Call::Environment {
        auth: fixtures.auth(None),
        params_api_key: Some(&key),
    })
    .expect("the environment call returns headers") else {
        panic!("the environment call returns headers");
    };
    assert_eq!(headers.get("api-key").map(String::as_str), Some("key-1"));
}

fn video_environment_rows() {
    let fixtures = Fixtures::new();
    let key = SecretString::from("key-1");
    let mut params_key: Option<SecretString> = None;
    let Outcome::VideoHeaders {
        headers,
        params_api_key,
    } = outcome(Call::VideoEnvironment {
        auth: fixtures.auth(None),
        api_key: Some(&key),
        params_api_key: &mut params_key,
    })
    .expect("the video environment call returns headers")
    else {
        panic!("the video environment call returns headers");
    };
    assert_eq!(headers.get("api-key").map(String::as_str), Some("key-1"));
    assert_eq!(
        params_api_key.as_deref(),
        Some("key-1"),
        "a blank params key is filled in place"
    );

    let mut kept: Option<SecretString> = Some(SecretString::from("params-key"));
    let Outcome::VideoHeaders { headers, .. } = outcome(Call::VideoEnvironment {
        auth: fixtures.auth(None),
        api_key: Some(&key),
        params_api_key: &mut kept,
    })
    .expect("the video environment call returns headers") else {
        panic!("the video environment call returns headers");
    };
    assert_eq!(
        headers.get("api-key").map(String::as_str),
        Some("params-key"),
        "the params key wins over the positional one"
    );
}

fn video_rows() {
    let options: VideoCreate = parse(
        r#"{"model":"sora-2","prompt":"a cat","seconds":"8","size":"720x1280","user":"u1","input_reference":"file-1","characters":[{"id":"c"}]}"#,
    );
    let Outcome::VideoParams(mapped) =
        outcome(Call::VideoParams(&options)).expect("the video params map")
    else {
        panic!("the video params call returns the mapped params");
    };
    assert_eq!(
        mapped.get("prompt").and_then(|value| value.as_str()),
        Some("a cat")
    );
    assert_eq!(
        mapped.get("seconds").and_then(|value| value.as_str()),
        Some("8")
    );
    assert_eq!(
        mapped.get("characters"),
        Some(&serde_json::json!([{"id": "c"}]))
    );

    let Outcome::VideoSupported(supported) =
        outcome(Call::VideoSupported).expect("the supported list reads")
    else {
        panic!("the supported call returns the parameter list");
    };
    assert_eq!(supported.len(), 7_usize);
    assert!(supported.contains(&"model"));
    assert!(!supported.contains(&"characters"));
}

fn failure_rows() {
    let error = leaf_error(super::block_on(dispatch(
        AzureResourceOperation::ResourceUrls(Box::new(resource_urls::Dispatch {
            call: Call::Url {
                resource: AzureResource::VectorStore,
                request: UrlRequest {
                    api_base: None,
                    fallback_api_base: None,
                    api_version: None,
                },
            },
        })),
    )));
    assert!(matches!(
        error,
        AzureResourceError::ResourceUrls(ResourceUrlError::Url(_))
    ));
}

pub(super) fn check() {
    container_url_rows();
    video_vector_url_rows();
    environment_headers_row();
    video_environment_rows();
    video_rows();
    failure_rows();
}
