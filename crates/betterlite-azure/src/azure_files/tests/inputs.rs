use std::time::Duration;

use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::*;

const DEFAULT_VERSION: &str = "2025-02-01-preview";
const DEPLOYMENT_BASE: &str = "https://fixture.test/openai/deployments/dep";

fn upload() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: None,
            content_type: None,
            data: b"plain".to_vec(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn client_view(step: Step) -> ClientView {
    let Outcome::Client(view) = run(step).outcome else {
        panic!("a client step returns the client");
    };
    view
}

fn raised(step: Step) -> Raised {
    let Outcome::Raised(raised) = run(step).outcome else {
        panic!("the step raises");
    };
    raised
}

fn retrieve(file_id: &str, reply: Planned) -> Step {
    Step {
        file_id: Some(file_id.to_owned()),
        reply: Some(reply),
        ..step(Action::Retrieve)
    }
}

fn params_with(timeout: f64, max_retries: i64) -> LiteLlmParams {
    LiteLlmParams {
        timeout: Some(timeout),
        max_retries: Some(max_retries),
        ..LiteLlmParams::default()
    }
}

fn azure_file_client_inputs() {
    let defaults = client_view(step(Action::Client));
    assert_eq!(defaults.api_key, KEY);
    assert_eq!(defaults.base_url, format!("{ENDPOINT}/openai/"));
    assert_eq!(defaults.api_version.as_deref(), Some(VERSION));
    assert_eq!(defaults.auth, AuthKind::ApiKey);
    assert_eq!(defaults.timeout, Duration::from_secs(600));
    assert_eq!(defaults.max_retries, 2);
    assert_eq!(defaults.kind, ClientKind::AzureOpenAi);
    assert_eq!(
        defaults.query,
        vec![("api-version".to_owned(), VERSION.to_owned())]
    );

    // the handler's own timeout and max_retries arguments never reach the client; litellm_params values do
    let mut tuned_step = step(Action::Client);
    tuned_step.params = params_with(7.5, 7);
    let tuned = client_view(tuned_step);
    assert_eq!(tuned.timeout, Duration::from_millis(7500));
    assert_eq!(tuned.max_retries, 7);

    let mut asynchronous = step(Action::Client);
    asynchronous.is_async = true;
    assert_eq!(client_view(asynchronous).kind, ClientKind::AsyncAzureOpenAi);

    // a missing api_version resolves through the configured default
    let mut unversioned = step(Action::Client);
    unversioned.settings.api_version = None;
    let unversioned = client_view(unversioned);
    assert_eq!(unversioned.api_version.as_deref(), Some(DEFAULT_VERSION));
    assert_eq!(
        unversioned.query,
        vec![("api-version".to_owned(), DEFAULT_VERSION.to_owned())]
    );

    // the v1 family is the standard OpenAI client: no api-version query and a bearer token
    let mut v1 = step(Action::Client);
    v1.settings.api_version = Some("v1".to_owned());
    let v1 = client_view(v1);
    assert_eq!(v1.base_url, format!("{ENDPOINT}/openai/v1/"));
    assert_eq!(v1.api_version, None);
    assert_eq!(v1.auth, AuthKind::Bearer);
    assert_eq!(v1.api_key, KEY);
    assert!(v1.query.is_empty());

    // an api_base holding a deployments path becomes the client's own base URL
    let mut deployment = step(Action::Client);
    deployment.settings.api_base = Some(DEPLOYMENT_BASE.to_owned());
    let deployment = client_view(deployment);
    assert_eq!(deployment.base_url, format!("{DEPLOYMENT_BASE}/"));

    // the ad token from litellm_params wins over the api key
    let mut token_step = step(Action::Client);
    token_step.params = LiteLlmParams {
        azure_ad_token: Some(SecretString::from("ad-token".to_owned())),
        ..LiteLlmParams::default()
    };
    let token = client_view(token_step);
    assert_eq!(token.auth, AuthKind::Bearer);
    assert_eq!(token.api_key, "ad-token");

    for kind in Supplied::ALL {
        let mut supplied = step(Action::Client);
        supplied.client = Some(kind);
        let observation = run(supplied);
        assert_eq!(observation.seen, vec![kind.kind()]);
        let Outcome::Client(view) = observation.outcome else {
            panic!("a supplied client comes back as it is");
        };
        assert_eq!(view.api_key, SUPPLIED_KEY);
        assert_eq!(view.timeout, Duration::from_secs(600));
        assert_eq!(view.max_retries, 0);
        assert_eq!(view.kind, kind.kind());
        match kind {
            Supplied::AzureOpenAi | Supplied::AsyncAzureOpenAi => {
                assert_eq!(view.base_url, format!("{SUPPLIED_ENDPOINT}/openai/"));
                assert_eq!(view.api_version.as_deref(), Some(SUPPLIED_VERSION));
                assert_eq!(view.auth, AuthKind::ApiKey);
            }
            Supplied::OpenAi | Supplied::AsyncOpenAi => {
                assert_eq!(view.base_url, format!("{SUPPLIED_ENDPOINT}/v1/"));
                assert_eq!(view.api_version, None);
                assert_eq!(view.auth, AuthKind::Bearer);
            }
        }
    }

    // a client the cache hands back is materialized and used, as LiteLLM returns its cached client
    let mut cached = step(Action::Client);
    cached.cache_hit = Some(ClientKind::OpenAi);
    let observation = run(cached);
    assert_eq!(observation.seen, vec![ClientKind::OpenAi]);
    let Outcome::Client(view) = observation.outcome else {
        panic!("the cached client comes back");
    };
    assert_eq!(view.base_url, format!("{SUPPLIED_ENDPOINT}/v1/"));

    // an empty v1 key carries no credential, so the client sends no auth header
    let mut empty_v1 = step(Action::Client);
    empty_v1.settings.api_version = Some("v1".to_owned());
    empty_v1.settings.api_key = Some(SecretString::from(String::new()));
    assert_eq!(client_view(empty_v1).auth, AuthKind::None);

    // a base URL that already carries api-version has it replaced rather than doubled
    let files = FilesClient {
        api_key: SecretString::from(KEY.to_owned()),
        base_url: Url::parse("https://fixture.test/openai/?api-version=old").expect("a URL"),
        organization: None,
        timeout: DEFAULT_TIMEOUT,
        max_retries: 0,
    };
    let client = SelectedClient::new(
        files,
        FileAuth::None,
        Some(VERSION.to_owned()),
        ClientKind::AzureOpenAi,
    )
    .expect("the client builds");
    assert_eq!(
        client.query(),
        vec![("api-version".to_owned(), VERSION.to_owned())]
    );
}

fn azure_file_client_errors() {
    // a builder that answers with no client
    let mut missing = step(Action::Create);
    missing.client = Some(Supplied::AzureOpenAi);
    missing.builder = BuilderMode::None;
    missing.data = Some(upload());
    let raised = raised(missing);
    assert_eq!(raised.class, "ValueError");
    assert_eq!(
        raised.message.as_deref(),
        Some(
            "AzureOpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
        )
    );

    // an async call with a synchronous client
    let mut wrong_async = retrieve("file-1", json_reply(FILE_JSON));
    wrong_async.is_async = true;
    wrong_async.client = Some(Supplied::AzureOpenAi);
    let raised = raised(wrong_async);
    assert_eq!(raised.class, "ValueError");
    assert_eq!(
        raised.message.as_deref(),
        Some(
            "AzureOpenAI client is not an instance of AsyncAzureOpenAI. Make sure you passed an AsyncAzureOpenAI client."
        )
    );

    // an async call with an async client goes through
    let mut async_client = retrieve("file-1", json_reply(FILE_JSON));
    async_client.is_async = true;
    async_client.client = Some(Supplied::AsyncAzureOpenAi);
    assert!(matches!(run(async_client).outcome, Outcome::Object(_)));

    // the sync path has no guard, so an async client still sends
    let mut sync_call = retrieve("file-1", json_reply(FILE_JSON));
    sync_call.client = Some(Supplied::AsyncAzureOpenAi);
    let observation = run(sync_call);
    assert!(matches!(observation.outcome, Outcome::Object(_)));
    assert_eq!(observation.seen, vec![ClientKind::AsyncAzureOpenAi]);

    // the SDK's status error travels without a catch
    let raised = raised(retrieve("file-1", reply(404, STATUS_ERROR)));
    assert_eq!(
        raised,
        Raised {
            class: "APIStatusError",
            message: Some(format!("Error code: 404 - {STATUS_ERROR}")),
            status: Some(404),
            error_type: Some("invalid_request_error".to_owned()),
            param: Some("file_id".to_owned()),
            code: Some("not_found".to_owned()),
        }
    );

    // no credential at all is the SDK's construction error
    let mut credentialless = step(Action::Create);
    credentialless.settings.api_key = None;
    credentialless.data = Some(upload());
    let raised = raised(credentialless);
    assert_eq!(raised.class, "OpenAIError");
    assert_eq!(
        raised.message.as_deref(),
        Some(
            "Missing credentials. Please pass one of `api_key`, `azure_ad_token`, `azure_ad_token_provider`, or the `AZURE_OPENAI_API_KEY` or `AZURE_OPENAI_AD_TOKEN` environment variables."
        )
    );

    // an explicit empty key fails before a request is sent
    let mut empty_key = step(Action::Create);
    empty_key.settings.api_key = Some(SecretString::from(String::new()));
    empty_key.data = Some(upload());
    let raised = raised(empty_key);
    assert_eq!(raised.class, "ValueError");
    assert_eq!(raised.message.as_deref(), Some("Unable to handle auth"));

    // a provider that answers with something the SDK refuses
    let mut non_text = retrieve("file-1", json_reply(FILE_JSON));
    non_text.provider = Some(callable(ProviderReply::NonText("None".to_owned())));
    let raised = raised(non_text);
    assert_eq!(raised.class, "ValueError");
    assert_eq!(
        raised.message.as_deref(),
        Some("Expected `azure_ad_token_provider` argument to return a string but it returned None")
    );

    let mut empty_token = retrieve("file-1", json_reply(FILE_JSON));
    empty_token.provider = Some(callable(ProviderReply::Text(String::new())));
    let raised = raised(empty_token);
    assert_eq!(
        raised.message.as_deref(),
        Some("Expected `azure_ad_token_provider` argument to return a string but it returned ")
    );

    // a provider that fails keeps its own error
    let mut failing = retrieve("file-1", json_reply(FILE_JSON));
    failing.provider = Some(callable(ProviderReply::Failure));
    let raised = raised(failing);
    assert_eq!(raised.class, "ValueError");
    assert_eq!(raised.message.as_deref(), Some("the token call failed"));

    // a provider token wins over the api key and rides on the request
    let mut provided = retrieve("file-1", json_reply(FILE_JSON));
    provided.provider = Some(callable(ProviderReply::Text("provider-token".to_owned())));
    let observation = run(provided);
    let request = only_call(&observation);
    assert_eq!(
        header(request, "authorization"),
        Some("Bearer provider-token")
    );
    assert_eq!(header(request, "api-key"), None);

    // the ad token wins over the provider: the SDK only calls the provider without a static token
    let mut static_token = retrieve("file-1", json_reply(FILE_JSON));
    static_token.provider = Some(callable(ProviderReply::Text("provider-token".to_owned())));
    static_token.params = LiteLlmParams {
        azure_ad_token: Some(SecretString::from("ad-token".to_owned())),
        ..LiteLlmParams::default()
    };
    let request = only_call(&run(static_token));
    assert_eq!(header(request, "authorization"), Some("Bearer ad-token"));

    // without a base URL the client cannot be built
    let mut homeless = retrieve("file-1", json_reply(FILE_JSON));
    homeless.settings.api_base = None;
    let raised = raised(homeless);
    assert_eq!(raised.class, "ValueError");
    assert_eq!(
        raised.message.as_deref(),
        Some(
            "Must provide one of the `base_url` or `azure_endpoint` arguments, or the `AZURE_OPENAI_ENDPOINT` environment variable"
        )
    );

    // an empty file id is the SDK's own argument check
    let raised = raised(retrieve("", json_reply(FILE_JSON)));
    assert_eq!(raised.class, "ValueError");
    assert_eq!(
        raised.message.as_deref(),
        Some("Expected a non-empty value for `file_id` but received ''")
    );
}

fn azure_file_routes() {
    // the deployment base keeps its path before the resource path
    let mut deployment = step(Action::Create);
    deployment.settings.api_base = Some(DEPLOYMENT_BASE.to_owned());
    deployment.data = Some(upload());
    deployment.reply = Some(json_reply(FILE_JSON));
    let request = only_call(&run(deployment));
    assert_eq!(request.url.path(), "/openai/deployments/dep/files");
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), VERSION.to_owned())]
    );

    // the v1 family sends to the standard OpenAI paths with a bearer token and no version
    let request = only_call(&run(retrieve("file two", json_reply(FILE_JSON))));
    assert_eq!(request.url.path(), "/openai/files/file%20two");
    let mut v1 = retrieve("file two", json_reply(FILE_JSON));
    v1.settings.api_version = Some("v1".to_owned());
    let request = only_call(&run(v1));
    assert_eq!(request.url.path(), "/openai/v1/files/file%20two");
    assert_eq!(header(request, "authorization"), Some("Bearer key-1"));
    assert_eq!(header(request, "api-key"), None);
    assert!(request.query.is_empty());

    // a supplied Azure client keeps its own base, version and key
    let mut supplied = step(Action::List);
    supplied.client = Some(Supplied::AzureOpenAi);
    supplied.purpose = Some("batch".to_owned());
    supplied.reply = Some(json_reply(PAGE_JSON));
    let request = only_call(&run(supplied));
    assert_eq!(request.url.host_str(), Some("supplied.test"));
    assert_eq!(request.url.path(), "/openai/files");
    assert_eq!(header(request, "api-key"), Some(SUPPLIED_KEY));
    assert_eq!(
        request.query,
        vec![
            ("api-version".to_owned(), SUPPLIED_VERSION.to_owned()),
            ("purpose".to_owned(), "batch".to_owned()),
        ]
    );

    // a supplied OpenAI client is used as it is: no Azure prefix and no version
    let mut plain = step(Action::Delete);
    plain.client = Some(Supplied::OpenAi);
    plain.file_id = Some("file-1".to_owned());
    plain.reply = Some(json_reply(r#""""#));
    let observation = run(plain);
    assert_eq!(
        observation.outcome,
        Outcome::Deleted(synthesized_record("file-1"))
    );
    let request = only_call(&observation);
    assert_eq!(request.url.path(), "/v1/files/file-1");
    assert_eq!(
        header(request, "authorization"),
        Some("Bearer supplied-key")
    );
    assert!(request.query.is_empty());

    // an async call reaches the same route
    let mut asynchronous = step(Action::Content);
    asynchronous.is_async = true;
    asynchronous.content = Some(FileContentRequest {
        file_id: "file two".to_owned(),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    });
    asynchronous.reply = Some(reply(200, "file-bytes"));
    let request = only_call(&run(asynchronous));
    assert_eq!(request.url.path(), "/openai/files/file%20two/content");
    assert_eq!(header(request, "accept"), Some("application/binary"));
}

fn synthesized_record(file_id: &str) -> FileDeleted {
    FileDeleted {
        id: file_id.to_owned(),
        object: FileKind::File,
        deleted: true,
    }
}
