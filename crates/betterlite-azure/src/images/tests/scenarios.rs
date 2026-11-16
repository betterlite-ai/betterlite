use std::collections::BTreeMap;

use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::family::ImageFamily;

use super::support::{
    API_KEY, AZURE, BEARER, Harness, MAI_BODY, PROMPT, ProviderScript, Scenario, VERSION,
    edit_form, ok_reply, reply, scenario,
};
use crate::credentials::selection::AzureTokenError;
use crate::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_image_edit_request_data, get_complete_url,
    validate_environment,
};
use crate::images::error::ImageError;
use crate::images::selection::{AzureImageConfig, get_azure_image_generation_config};
use crate::images::target::{AzureClientParams, create_azure_base_url};

pub(super) fn deployment_body_drops_model() {
    let data = edit_form(Some("dall-e-3"));
    let deployment_edits =
        format!("{AZURE}/openai/deployments/dall-e-3/images/edits?api-version={VERSION}");
    assert!(!finalize_image_edit_request_data(&data, &deployment_edits).contains_key("model"));
    let edits = format!("{AZURE}/openai/images/edits");
    assert!(finalize_image_edit_request_data(&data, &edits).contains_key("model"));
    let generations = format!("{AZURE}/openai/deployments/dall-e-3/images/generations");
    assert!(finalize_image_edit_request_data(&data, &generations).contains_key("model"));

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
        assert_eq!(
            get_azure_image_generation_config(model),
            expected,
            "{model:?}"
        );
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
    let reply = &outcome.success().reply;
    assert_eq!(reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(reply.quality.as_deref(), Some("hd"));
    assert_eq!(reply.output_format.as_deref(), Some("png"));

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        optional: r#"{"size":"1792x1024","quality":"standard"}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    let reply = &outcome.success().reply;
    assert_eq!(reply.size.as_deref(), Some("1792x1024"));
    assert_eq!(reply.quality.as_deref(), Some("standard"));

    let mut harness = Harness::new(&[]).with_reply(reply(200, MAI_BODY));
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        ..scenario("MAI-Image-1")
    });
    assert_eq!(
        outcome.url(),
        format!("{AZURE}/mai/v1/images/generations?api-version={VERSION}")
    );
    assert_eq!(outcome.failure().class(), "AzureOpenAIError");
    assert_eq!(outcome.failure().status(), Some(500));
    assert!(outcome.failure().to_string().contains("B554"));
    assert_eq!(outcome.logger.post.len(), 1);
}

fn sync_transform() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&scenario("dall-e-3"));
    let reply = &outcome.success().reply;
    assert_eq!(reply.size, None);
    assert_eq!(reply.quality, None);
    assert_eq!(reply.output_format, None);
    assert_eq!(outcome.logger.post.len(), 1);
    assert_eq!(outcome.logger.post[0].input, PROMPT);

    let mut harness = Harness::new(&[]).with_reply(reply(200, MAI_BODY));
    let outcome = harness.run(&scenario("MAI-Image-1"));
    assert_eq!(
        outcome.url(),
        format!("{AZURE}/mai/v1/images/generations?api-version={VERSION}")
    );
    assert_eq!(outcome.failure().class(), "AzureOpenAIError");
    assert_eq!(outcome.failure().status(), Some(500));
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
        validate_environment(&EditAuth {
            headers: self.headers.into_iter().collect(),
            api_key: self.argument,
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
}

// the deployment-settings key fills in when neither the argument nor litellm_params carries one
fn edit_credentials_key() {
    let credentials = AzureCredentials {
        api_key: Some(secrecy::SecretString::from("credential-key")),
        ..AzureCredentials::default()
    };
    let environment = BTreeMap::new();
    let keyed = EditCase::new(&credentials, &environment)
        .run()
        .expect("credential key");
    assert_eq!(
        keyed.get("api-key").map(String::as_str),
        Some("credential-key")
    );
}

fn edit_key_and_token() {
    let credentials = AzureCredentials::default();
    let environment = BTreeMap::new();
    let keyed = EditCase {
        params: EditParams {
            api_key: Some(secrecy::SecretString::from(API_KEY)),
            ..EditParams::default()
        },
        ..EditCase::new(&credentials, &environment)
    }
    .run()
    .expect("api-key auth");
    assert_eq!(keyed.get("api-key").map(String::as_str), Some(API_KEY));
    assert!(!keyed.contains_key("Authorization"));

    let token = EditParams {
        azure_ad_token: Some(secrecy::SecretString::from(BEARER)),
        ..EditParams::default()
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
            ..EditParams::default()
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
        ..AzureCredentials::default()
    };
    assert_eq!(
        edit_url(Some(""), Some(""), &global),
        "https://global.openai.azure.com/openai/deployments/dall-e-3/images/edits?api-version=2024-06-01"
    );
    let empty_version = AzureCredentials {
        api_base: Some("https://global.openai.azure.com".to_owned()),
        api_version: Some(String::new()),
        ..AzureCredentials::default()
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
        ..AzureCredentials::default()
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

pub(super) fn azure_image_body_and_retry() {
    body_table();
    retry_table();
}

fn body_table() {
    body_model_table();
    body_options_table();
}

fn body_model_table() {
    body_absent_model();
    body_base_model();
}

fn body_absent_model() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        model: Some(String::new()),
        ..scenario("dall-e-3")
    });
    // the deployment route drops the model from the wire, so the logged request data carries it
    assert_eq!(outcome.logger.pre[0].data.get("model"), Some(&Json::Null));
    assert!(outcome.calls[0].body_field("model").is_none());
    assert_eq!(outcome.success().hidden_model, None);
}

fn body_base_model() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        base_model: Some("hidden-dall-e-3".to_owned()),
        ..scenario("my-deployment")
    });
    assert_eq!(
        outcome.logger.pre[0].data.get("model"),
        Some(&Json::Text("hidden-dall-e-3".to_owned()))
    );
    assert!(outcome.calls[0].body_field("model").is_none());
    assert_eq!(
        outcome.success().hidden_model.as_deref(),
        Some("hidden-dall-e-3")
    );
    assert_eq!(
        outcome.url(),
        format!(
            "{AZURE}/openai/deployments/my-deployment/images/generations?api-version={VERSION}"
        )
    );

    // an empty base model counts as absent, so the deployment's own name fills the request data
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        base_model: Some(String::new()),
        ..scenario("my-deployment")
    });
    assert_eq!(
        outcome.logger.pre[0].data.get("model"),
        Some(&Json::Text("my-deployment".to_owned()))
    );
}

fn body_options_table() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        optional: r#"{"size":"1024x1024","extra_body":{"size":"512x512","foo":"bar"}}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.calls[0].body_field("size"),
        Some(&Json::Text("512x512".to_owned()))
    );
    assert_eq!(
        outcome.calls[0].body_field("foo"),
        Some(&Json::Text("bar".to_owned()))
    );
    assert!(outcome.calls[0].body_field("extra_body").is_none());
    assert_eq!(
        outcome.calls[0].body_field("prompt"),
        Some(&Json::Text(PROMPT.to_owned()))
    );
}

fn retry_table() {
    retry_accepted();
    retry_rejected();
}

fn retry_accepted() {
    for accepted in [r#"{"max_retries":3}"#, r#"{"max_retries":true}"#] {
        let mut harness = Harness::new(&[]).with_reply(ok_reply());
        let outcome = harness.run(&Scenario {
            optional: accepted.to_owned(),
            ..scenario("dall-e-3")
        });
        assert!(outcome.result.is_ok(), "{accepted}");
        assert!(outcome.calls[0].body_field("max_retries").is_none());
    }
}

fn retry_rejected() {
    for rejected in [
        r#"{"max_retries":null}"#,
        r#"{"max_retries":"3"}"#,
        r#"{"max_retries":3.5}"#,
    ] {
        let mut harness = Harness::new(&[]).with_reply(ok_reply());
        let outcome = harness.run(&Scenario {
            optional: rejected.to_owned(),
            ..scenario("dall-e-3")
        });
        let error = outcome.failure();
        assert_eq!(error.class(), "AzureOpenAIError", "{rejected}");
        assert_eq!(error.status(), Some(422), "{rejected}");
        assert_eq!(
            error.to_string(),
            "max retries must be an int",
            "{rejected}"
        );
        // LiteLLM re-raises its own image error, so the sync leg must not wrap this one again
        assert!(
            matches!(outcome.result, Err(ImageError::MaxRetries)),
            "{rejected}"
        );
        assert!(outcome.calls.is_empty(), "{rejected}");
        assert!(outcome.logger.post.is_empty(), "{rejected}");
    }
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        optional: r#"{"extra_body":"nope"}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.failure().class(), "AzureOpenAIError");
    assert_eq!(outcome.failure().status(), Some(500));
}

pub(super) fn azure_image_auth() {
    auth_table();
    provider_table();
}

fn auth_table() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&scenario("dall-e-3"));
    assert_eq!(outcome.header("api-key"), Some(API_KEY));
    assert_eq!(outcome.header("authorization"), None);
    assert_eq!(
        outcome.pre_headers().get("api-key"),
        Some(&Json::Text("***REDACTED***".to_owned()))
    );
    assert_eq!(outcome.constructed, 1);

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_key: None,
        azure_ad_token: Some(BEARER.to_owned()),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.header("authorization"),
        Some(format!("Bearer {BEARER}").as_str())
    );
    assert_eq!(
        outcome.pre_headers().get("authorization"),
        Some(&Json::Text("***REDACTED***".to_owned()))
    );

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        azure_ad_token: Some(BEARER.to_owned()),
        params: r#"{"azure_ad_token":"params-token"}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.header("authorization"),
        Some(format!("Bearer {BEARER}").as_str())
    );
}

fn provider_table() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_key: None,
        provider: Some(ProviderScript::Token("provider-token")),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.header("authorization"),
        Some("Bearer provider-token")
    );

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        headers: vec![("API-KEY".to_owned(), "custom".to_owned())],
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.header("api-key"), Some("custom"));
    assert_eq!(outcome.sent_headers().len(), 1);

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_key: None,
        provider: Some(ProviderScript::Failure("RuntimeError", "boom")),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.failure().class(), "AzureOpenAIError");
    assert_eq!(outcome.failure().status(), Some(500));
    assert!(outcome.calls.is_empty());
}

pub(super) fn azure_image_errors() {
    sync_failure_table();
    async_failure_table();
}

fn sync_failure_table() {
    let body = r#"{"error":{"message":"boom","code":400}}"#;
    let mut harness = Harness::new(&[]).with_reply(reply(400, body));
    let outcome = harness.run(&scenario("dall-e-3"));
    let error = outcome.failure();
    assert_eq!(error.class(), "AzureOpenAIError");
    assert_eq!(error.status(), Some(400));
    assert_eq!(outcome.logger.post.len(), 1);
    assert_eq!(outcome.logger.post[0].input, PROMPT);

    let mut harness = Harness::new(&[]).with_reply(reply(200, "not json"));
    let outcome = harness.run(&scenario("dall-e-3"));
    assert_eq!(outcome.failure().class(), "AzureOpenAIError");
    assert_eq!(outcome.failure().status(), Some(500));
    assert!(outcome.logger.post.is_empty());

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        optional: r#"{"max_retries":null}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.failure().status(), Some(422));
    assert!(outcome.calls.is_empty());
    assert!(outcome.logger.pre.is_empty());
}

fn async_failure_table() {
    let body = r#"{"error":{"message":"boom","code":400}}"#;
    let mut harness = Harness::new(&[]).with_reply(reply(400, body));
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.failure().class(), "AzureOpenAIError");
    assert_eq!(outcome.failure().status(), Some(400));
    assert_eq!(outcome.logger.post.len(), 2);
    match &outcome.logger.post[1].original_response {
        Json::Text(text) => assert!(text.contains("boom")),
        _ => panic!("the async leg logs the failure as text"),
    }

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        delegate_async: true,
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome.success().reply.size.as_deref(), Some("1024x1024"));
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
