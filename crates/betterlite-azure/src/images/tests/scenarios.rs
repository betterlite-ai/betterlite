use std::collections::BTreeMap;

use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::family::ImageFamily;
use secrecy::ExposeSecret as _;

use super::support::{
    API_KEY, AZURE, BEARER, Harness, MAI_BODY, PROMPT, RecordingSources, Scenario, VERSION,
    edit_form, ok_reply, reply, scenario,
};
use crate::azure_urls::AuthParams;
use crate::credentials::selection::AzureTokenError;
use crate::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_edit_data, get_complete_url, validate_environment,
};
use crate::images::selection::{AzureImageConfig, image_config};
use crate::images::target::{AzureClientParams, create_azure_base_url};

pub(super) fn deployment_body_drops_model() {
    let data = edit_form(Some("dall-e-3"));
    let deployment_edits =
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?api-version={VERSION}");
    assert!(!finalize_edit_data(&data, &deployment_edits).contains_key("model"));
    let edits = format!("{AZURE}/openai/images/edits");
    assert!(finalize_edit_data(&data, &edits).contains_key("model"));
    let generations = format!("{AZURE}/openai/deployments/dall-e-3/images/generations");
    assert!(finalize_edit_data(&data, &generations).contains_key("model"));

    let deployment = format!("{AZURE}/openai/deployments/dall-e-3");
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_base: Some(deployment.clone()),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.url(),
        format!("{deployment}/images/generations?api-version={VERSION}")
    );
    assert!(outcome.calls[0].body_field("model").is_none());
    assert_eq!(outcome.success().reply.data.len(), 1);
}

pub(super) fn v1_keeps_model() {
    let v1 = format!("{AZURE}/openai/v1/images/generations?api-version=preview");
    assert_eq!(
        target(Some(AZURE), Some("preview"), Some("dall-e-3"), None),
        v1
    );
    let trailing = format!("{AZURE}/");
    assert_eq!(
        target(Some(trailing.as_str()), Some("v1"), Some("dall-e-3"), None),
        format!("{AZURE}/openai/v1/images/generations?api-version=v1")
    );

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_version: Some("preview".to_owned()),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.url(), v1);
    assert_eq!(
        outcome.calls[0].body_field("model"),
        Some(&Json::Text("dall-e-3".to_owned()))
    );
}

pub(super) fn dalle2_dalle3_gpt_selection() {
    selection_table();
    async_transforms();
    sync_transform();
}

fn selection_table() {
    let cases = [
        (None, AzureImageConfig::DallE2),
        (Some(""), AzureImageConfig::DallE2),
        (Some("DALL-E-2"), AzureImageConfig::DallE2),
        (Some("azure/dall_e2"), AzureImageConfig::DallE2),
        (Some("DALL-E-3"), AzureImageConfig::DallE3),
        (Some("azure/azure-dall-e-3"), AzureImageConfig::DallE3),
        (Some("MAI-Image-1"), AzureImageConfig::Mai),
        (Some("azure_ai/mai_image_1"), AzureImageConfig::Mai),
        (Some("gpt-image-1"), AzureImageConfig::GptImage),
        (Some("azure/gpt-4o"), AzureImageConfig::GptImage),
    ];
    for (model, expected) in cases {
        assert_eq!(image_config(model), expected, "{model:?}");
    }
    assert_eq!(AzureImageConfig::Mai.family(), None);
    assert_eq!(AzureImageConfig::DallE3.family(), Some(ImageFamily::DallE3));
    assert_eq!(
        AzureImageConfig::GptImage.class_name(),
        "AzureGPTImageGenerationConfig"
    );
}

fn async_transforms() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        ..scenario("dall-e-3")
    });
    let image = &outcome.success().reply;
    assert_eq!(image.size.as_deref(), Some("1024x1024"));
    assert_eq!(image.quality.as_deref(), Some("hd"));
    assert_eq!(image.output_format.as_deref(), Some("png"));

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        optional: r#"{"size":"1792x1024","quality":"standard"}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    let image = &outcome.success().reply;
    assert_eq!(image.size.as_deref(), Some("1792x1024"));
    assert_eq!(image.quality.as_deref(), Some("standard"));

    let mut harness = Harness::new(&[]).with_reply(reply(200, MAI_BODY));
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        ..scenario("MAI-Image-1")
    });
    assert_eq!(
        outcome.url(),
        format!("{AZURE}/mai/v1/images/generations?api-version={VERSION}")
    );
    // the async leg re-raises its own error unwrapped
    assert_eq!(outcome.failure().classification(), ("NotBuilt", None));
    assert!(outcome.failure().to_string().contains("B554"));
    assert_eq!(outcome.logger.post.len(), 1);
}

fn sync_transform() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&scenario("dall-e-3"));
    let image = &outcome.success().reply;
    assert_eq!(image.size, None);
    assert_eq!(image.quality, None);
    assert_eq!(image.output_format, None);
    assert_eq!(outcome.logger.post.len(), 1);
    assert_eq!(outcome.logger.post[0].input, PROMPT);

    let mut harness = Harness::new(&[]).with_reply(reply(200, MAI_BODY));
    let outcome = harness.run(&scenario("MAI-Image-1"));
    assert_eq!(
        outcome.url(),
        format!("{AZURE}/mai/v1/images/generations?api-version={VERSION}")
    );
    assert_eq!(
        outcome.failure().classification(),
        ("AzureOpenAIError", Some(500))
    );
    assert!(outcome.failure().to_string().contains("B554"));
    assert!(outcome.logger.post.is_empty());
}

pub(super) fn image_edit_deployment_url() {
    edit_auth_table();
    edit_url_table();
}

struct EditCase<'data> {
    headers: Vec<(String, String)>,
    argument: Option<&'data str>,
    params: EditParams,
    credentials: &'data AzureCredentials,
    environment: &'data BTreeMap<String, String>,
}

impl<'data> EditCase<'data> {
    fn new(
        credentials: &'data AzureCredentials,
        environment: &'data BTreeMap<String, String>,
    ) -> Self {
        Self {
            headers: Vec::new(),
            argument: None,
            params: EditParams::default(),
            credentials,
            environment,
        }
    }

    fn run(self) -> Result<BTreeMap<String, String>, AzureTokenError> {
        let argument = self.argument.map(secrecy::SecretString::from);
        validate_environment(&EditAuth {
            headers: self.headers.into_iter().collect(),
            api_key: argument.as_ref(),
            params: &self.params,
            credentials: self.credentials,
            environment: self.environment,
            sources: &super::support::NoSources,
            refresh_enabled: false,
            provider: None,
        })
    }
}

fn edit_auth_table() {
    edit_key_and_token();
    edit_header_and_environment();
    edit_credentials_key();
    edit_client_secret_grant();
    edit_default_scope_grant();
}

fn client_secret_params(scope: Option<&str>) -> EditParams {
    EditParams {
        api_key: None,
        azure_ad_token: None,
        auth: AuthParams {
            tenant_id: Some("tenant-id".to_owned()),
            client_id: Some("client-id".to_owned()),
            client_secret: Some(secrecy::SecretString::from("client-secret")),
            azure_username: None,
            azure_password: None,
            azure_scope: scope.map(str::to_owned),
        },
    }
}

// litellm_params.auth reaches the credential sources as a client-secret grant with its scope
fn edit_client_secret_grant() {
    let credentials = AzureCredentials::default();
    let environment = BTreeMap::new();
    let sources = RecordingSources::new("recorded-token");
    let params = client_secret_params(Some("https://scope.example/.default"));
    let headers = validate_environment(&EditAuth {
        headers: BTreeMap::new(),
        api_key: None,
        params: &params,
        credentials: &credentials,
        environment: &environment,
        sources: &sources,
        refresh_enabled: false,
        provider: None,
    })
    .expect("client secret auth");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer recorded-token")
    );
    let recorded = sources.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].tenant_id, "tenant-id");
    assert_eq!(recorded[0].client_id, "client-id");
    assert_eq!(recorded[0].client_secret.expose_secret(), "client-secret");
    assert_eq!(recorded[0].scope, "https://scope.example/.default");
}

// an absent scope reaches the grant as the Azure default, not as an empty string
fn edit_default_scope_grant() {
    let credentials = AzureCredentials::default();
    let environment = BTreeMap::new();
    let sources = RecordingSources::new("recorded-token");
    let params = client_secret_params(None);
    validate_environment(&EditAuth {
        headers: BTreeMap::new(),
        api_key: None,
        params: &params,
        credentials: &credentials,
        environment: &environment,
        sources: &sources,
        refresh_enabled: false,
        provider: None,
    })
    .expect("default scope auth");
    assert_eq!(sources.recorded()[0].scope, DEFAULT_SCOPE);
}

// the deployment-settings key fills in when neither the argument nor litellm_params carries one
fn edit_credentials_key() {
    let credentials = AzureCredentials {
        api_base: None,
        api_version: None,
        api_key: Some(secrecy::SecretString::from("credential-key")),
    };
    let environment = BTreeMap::new();
    let keyed = EditCase::new(&credentials, &environment)
        .run()
        .expect("credential key");
    assert_eq!(
        keyed.get("api-key").map(String::as_str),
        Some("credential-key")
    );

    // LiteLLM fills the positional key only when params.api_key is None, so an empty one blocks it
    let blocked = EditCase {
        argument: Some("positional-key"),
        params: EditParams {
            api_key: Some(secrecy::SecretString::from("")),
            azure_ad_token: None,
            auth: AuthParams::default(),
        },
        ..EditCase::new(&credentials, &environment)
    }
    .run()
    .expect("credential key behind an empty params key");
    assert_eq!(
        blocked.get("api-key").map(String::as_str),
        Some("credential-key")
    );
}

fn edit_key_and_token() {
    let credentials = AzureCredentials::default();
    let environment = BTreeMap::new();
    let keyed = EditCase {
        params: EditParams {
            api_key: Some(secrecy::SecretString::from(API_KEY)),
            azure_ad_token: None,
            auth: AuthParams::default(),
        },
        ..EditCase::new(&credentials, &environment)
    }
    .run()
    .expect("api-key auth");
    assert_eq!(keyed.get("api-key").map(String::as_str), Some(API_KEY));
    assert!(!keyed.contains_key("Authorization"));

    let token = EditParams {
        api_key: None,
        azure_ad_token: Some(secrecy::SecretString::from(BEARER)),
        auth: AuthParams::default(),
    };
    let bearer = EditCase {
        params: token,
        ..EditCase::new(&credentials, &environment)
    }
    .run()
    .expect("token auth");
    assert_eq!(
        bearer.get("Authorization").map(String::as_str),
        Some(format!("Bearer {BEARER}").as_str())
    );
}

fn edit_header_and_environment() {
    let credentials = AzureCredentials::default();
    let environment = BTreeMap::new();
    let keyed_headers = EditCase {
        headers: vec![("api-key".to_owned(), "given".to_owned())],
        params: EditParams {
            api_key: Some(secrecy::SecretString::from(API_KEY)),
            azure_ad_token: None,
            auth: AuthParams::default(),
        },
        ..EditCase::new(&credentials, &environment)
    }
    .run()
    .expect("existing key");
    assert_eq!(
        keyed_headers.get("api-key").map(String::as_str),
        Some("given")
    );

    let env = BTreeMap::from([("AZURE_AD_TOKEN".to_owned(), "env-token".to_owned())]);
    let from_env = EditCase::new(&credentials, &env).run().expect("env token");
    assert_eq!(
        from_env.get("Authorization").map(String::as_str),
        Some("Bearer env-token")
    );
}

fn edit_url_table() {
    edit_url_requires_base();
    edit_url_versions();
    edit_url_empty_arguments();
}

// an empty request value counts as unset, so the deployment settings and then the default fill in
fn edit_url_empty_arguments() {
    let global = AzureCredentials {
        api_base: Some("https://global.openai.azure.com".to_owned()),
        api_version: Some("2024-06-01".to_owned()),
        api_key: None,
    };
    assert_eq!(
        edit_url(Some(""), Some(""), &global),
        "https://global.openai.azure.com/openai/deployments/dall-e-3/images/edits?api-version=2024-06-01"
    );
    let empty_version = AzureCredentials {
        api_base: Some("https://global.openai.azure.com".to_owned()),
        api_version: Some(String::new()),
        api_key: None,
    };
    assert_eq!(
        edit_url(None, None, &empty_version),
        format!(
            "https://global.openai.azure.com/openai/deployments/dall-e-3/images/edits?api-version={VERSION}"
        )
    );
}

fn edit_url_requires_base() {
    let credentials = AzureCredentials::default();
    let missing = get_complete_url(&EditUrl {
        model: "dall-e-3",
        api_base: None,
        api_version: None,
        credentials: &credentials,
        default_api_version: VERSION,
    })
    .expect_err("no api_base");
    assert_eq!(
        missing.to_string(),
        "api_base is required for Azure AI Studio. Please set the api_base parameter. Passed `api_base=None`"
    );
}

fn edit_url_versions() {
    let global = AzureCredentials {
        api_base: Some("https://global.openai.azure.com".to_owned()),
        api_version: Some("2024-06-01".to_owned()),
        api_key: None,
    };
    let url = get_complete_url(&EditUrl {
        model: "dall-e-3",
        api_base: None,
        api_version: None,
        credentials: &global,
        default_api_version: VERSION,
    })
    .expect("global base");
    assert_eq!(
        url,
        "https://global.openai.azure.com/openai/deployments/dall-e-3/images/edits?api-version=2024-06-01"
    );
    let url = get_complete_url(&EditUrl {
        model: "dall-e-3",
        api_base: Some(AZURE),
        api_version: Some("2024-05-01"),
        credentials: &global,
        default_api_version: VERSION,
    })
    .expect("request version");
    assert_eq!(
        url,
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?api-version=2024-05-01")
    );
}

pub(super) fn preview_image_path() {
    preview_targets();
    preview_request();
}

fn preview_targets() {
    let deployment = format!("{AZURE}/openai/deployments/my-deployment");
    assert_eq!(
        target(
            Some(deployment.as_str()),
            Some(VERSION),
            Some("my-deployment"),
            None
        ),
        format!("{deployment}/images/generations?api-version={VERSION}")
    );
    assert_eq!(
        target(Some(AZURE), Some(VERSION), Some("my-deployment"), None),
        format!(
            "{AZURE}/openai/deployments/my-deployment/images/generations?api-version={VERSION}"
        )
    );
    let queried = format!("{AZURE}?foo=1");
    assert_eq!(
        target(
            Some(queried.as_str()),
            Some(VERSION),
            Some("my-deployment"),
            None
        ),
        format!(
            "{queried}/openai/deployments/my-deployment/images/generations?api-version={VERSION}"
        )
    );
    assert_eq!(
        target(Some(AZURE), Some(VERSION), None, None),
        format!("{AZURE}/openai/deployments//images/generations?api-version={VERSION}")
    );
    assert_eq!(
        target(Some(AZURE), Some(""), Some("my-deployment"), None),
        format!("{AZURE}/openai/deployments/my-deployment/images/generations?api-version=")
    );
}

fn preview_request() {
    let deployment = format!("{AZURE}/openai/deployments/my-deployment");
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_base: Some(deployment.clone()),
        ..scenario("my-deployment")
    });
    assert_eq!(
        outcome.url(),
        format!("{deployment}/images/generations?api-version={VERSION}")
    );
}

pub(super) fn flux2_and_mai_target() {
    base_selection();
    flux2_table();
    mai_table();
    async_flux2_target();
}

// the async leg passes data's model on both sides, so an empty model argument still reaches FLUX 2
fn async_flux2_target() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        model: Some(String::new()),
        base_model: Some("FLUX.2-pro".to_owned()),
        delegate_async: true,
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.url(),
        format!("{AZURE}/providers/blackforestlabs/v1/flux-2-pro?api-version={VERSION}")
    );
}

fn base_selection() {
    let trailing = format!("{AZURE}//");
    assert_eq!(
        target(
            Some(trailing.as_str()),
            Some(VERSION),
            Some("dall-e-3"),
            None
        ),
        format!("{AZURE}/openai/deployments/dall-e-3/images/generations?api-version={VERSION}")
    );
    assert_eq!(
        target(None, Some(VERSION), None, None),
        format!("/openai/deployments//images/generations?api-version={VERSION}")
    );
    assert_eq!(
        target(Some(""), Some(VERSION), None, None),
        format!("/openai/deployments//images/generations?api-version={VERSION}")
    );
    let via_base_url = create_azure_base_url(
        &AzureClientParams {
            azure_endpoint: None,
            base_url: Some(AZURE),
            api_version: Some(VERSION),
        },
        Some("dall-e-3"),
        None,
    )
    .expect("base_url");
    assert_eq!(
        via_base_url,
        format!("{AZURE}/openai/deployments/dall-e-3/images/generations?api-version={VERSION}")
    );
}

fn flux2_table() {
    flux2_models();
    flux2_provider_bases();
}

fn flux2_models() {
    assert_eq!(
        target(
            Some(AZURE),
            Some(VERSION),
            Some("FLUX.2-pro"),
            Some("mai-image-1")
        ),
        format!("{AZURE}/mai/v1/images/generations?api-version={VERSION}")
    );
    assert_eq!(
        target(Some(AZURE), Some(""), Some("flux-2-pro"), None),
        format!("{AZURE}/providers/blackforestlabs/v1/flux-2-pro?api-version=preview")
    );
    assert_eq!(
        target(Some(AZURE), Some(VERSION), Some("FLUX.2-flex"), None),
        format!("{AZURE}/providers/blackforestlabs/v1/flux-2-flex?api-version={VERSION}")
    );
    assert_eq!(
        target(Some(AZURE), Some(VERSION), Some("flux-2-pro"), None),
        format!("{AZURE}/providers/blackforestlabs/v1/flux-2-pro?api-version={VERSION}")
    );
}

fn flux2_provider_bases() {
    let providers = format!("{AZURE}/providers/x");
    assert_eq!(
        target(
            Some(providers.as_str()),
            Some(VERSION),
            Some("flux.2-pro"),
            None
        ),
        format!("{providers}?api-version={VERSION}")
    );
    let with_query = format!("{AZURE}/providers/x?api-version=custom");
    assert_eq!(
        target(
            Some(with_query.as_str()),
            Some(VERSION),
            Some("flux.2-pro"),
            None
        ),
        with_query
    );
}

fn mai_table() {
    let mai = format!("{AZURE}/mai/v1/images/x?foo=1");
    assert_eq!(
        target(Some(mai.as_str()), Some(VERSION), Some("mai-image-1"), None),
        format!("{AZURE}/mai/v1/images/generations?foo=1")
    );
    assert_eq!(
        target(Some(AZURE), Some(""), Some("mai-image-1"), None),
        format!("{AZURE}/mai/v1/images/generations?api-version=preview")
    );
    // an empty base model counts as absent, so the deployment's own name decides the route
    assert_eq!(
        target(Some(AZURE), Some(VERSION), Some("mai-image-1"), Some("")),
        format!("{AZURE}/mai/v1/images/generations?api-version={VERSION}")
    );
}

pub(super) fn configured_base_query() {
    let credentials = AzureCredentials::default();
    let kept = format!("{AZURE}?api-version=2024-01-01");
    assert_eq!(
        edit_url(Some(kept.as_str()), None, &credentials),
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?api-version=2024-01-01")
    );
    let empty = format!("{AZURE}?api-version=");
    assert_eq!(
        edit_url(Some(empty.as_str()), None, &credentials),
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?api-version=")
    );
    let extra = format!("{AZURE}?foo=1");
    assert_eq!(
        edit_url(Some(extra.as_str()), None, &credentials),
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?foo=1&api-version={VERSION}")
    );
    let deployment = format!("{AZURE}/openai/deployments/dall-e-3");
    assert_eq!(
        edit_url(Some(deployment.as_str()), None, &credentials),
        format!("{deployment}?api-version={VERSION}")
    );
    let repeated = format!(
        "{AZURE}/openai/deployments/dall-e-3/images/edits?api-version=2024-01-01&api-version=2024-02-02"
    );
    assert_eq!(
        edit_url(Some(repeated.as_str()), None, &credentials),
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?api-version=2024-01-01")
    );
    assert_eq!(
        edit_url(Some(AZURE), Some("preview"), &credentials),
        format!("{AZURE}/openai/v1/images/edits?api-version=preview")
    );
}

fn target(
    base: Option<&str>,
    version: Option<&str>,
    model: Option<&str>,
    base_model: Option<&str>,
) -> String {
    create_azure_base_url(
        &AzureClientParams {
            azure_endpoint: base,
            base_url: None,
            api_version: version,
        },
        model,
        base_model,
    )
    .expect("a composed URL")
}

fn edit_url(
    api_base: Option<&str>,
    api_version: Option<&str>,
    credentials: &AzureCredentials,
) -> String {
    get_complete_url(&EditUrl {
        model: "dall-e-3",
        api_base,
        api_version,
        credentials,
        default_api_version: VERSION,
    })
    .expect("an edit URL")
}
