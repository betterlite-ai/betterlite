#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{
    ApiBaseContext, ApiBaseError, ApiBaseHooks, ApiBaseParams, LoginFreeProvider, get_api_base,
};
use crate::catalog::ModelCatalog;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::resolution::ResolutionContext;

#[derive(Clone, Copy)]
enum ClaudeHook {
    Records,
    Fails,
}

struct FakeHooks {
    login_free: Option<String>,
    claude: ClaudeHook,
}

impl ApiBaseHooks for FakeHooks {
    fn login_free_api_base(&self, provider: LoginFreeProvider) -> Option<String> {
        let slug = match provider {
            LoginFreeProvider::GithubCopilot => "github_copilot",
            LoginFreeProvider::Chatgpt => "chatgpt",
        };
        self.login_free
            .as_ref()
            .map(|base| format!("{base}:{slug}"))
    }

    fn vertex_claude_url(
        &self,
        location: &str,
        project: &str,
        model: &str,
        stream: bool,
    ) -> Result<String, ApiBaseError> {
        match self.claude {
            ClaudeHook::Records => Ok(format!("claude:{location}:{project}:{model}:{stream}")),
            ClaudeHook::Fails => Err(ApiBaseError::InvalidVertexLocation {
                message: format!("invalid vertex location {location}"),
            }),
        }
    }
}

fn lookup(
    model: &str,
    params: &ApiBaseParams,
    aliases: &BTreeMap<String, String>,
    hooks: &dyn ApiBaseHooks,
) -> Result<Option<String>, ApiBaseError> {
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let context = ApiBaseContext {
        model_alias_map: aliases,
        hooks,
        resolution: &resolution,
    };
    get_api_base(model, params, &context)
}

fn recorded_hooks() -> FakeHooks {
    FakeHooks {
        login_free: Some(String::from("https://login-free.test")),
        claude: ClaudeHook::Records,
    }
}

fn aliases(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn claude_error_hooks() -> FakeHooks {
    FakeHooks {
        login_free: None,
        claude: ClaudeHook::Fails,
    }
}

const GEMINI_PRO_GENERATE: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent";

#[rstest]
#[case::api_base_wins_over_base_url(
    Some("https://explicit.test"),
    Some("https://alias.test"),
    "https://explicit.test"
)]
#[case::an_empty_api_base_wins(Some(""), Some("https://alias.test"), "")]
#[case::base_url_is_used_without_api_base(None, Some("https://alias.test"), "https://alias.test")]
#[case::an_empty_base_url_falls_through(None, Some(""), GEMINI_PRO_GENERATE)]
fn precedence_params_before_the_lookup(
    #[case] api_base: Option<&str>,
    #[case] base_url: Option<&str>,
    #[case] expected: &str,
) {
    let params = ApiBaseParams {
        api_base: api_base.map(str::to_owned),
        base_url: base_url.map(str::to_owned),
        custom_llm_provider: None,
        api_key: None,
        vertex_location: None,
        vertex_project: None,
        stream: false,
    };
    assert_eq!(
        lookup(
            "gemini/gemini-pro",
            &params,
            &BTreeMap::new(),
            &recorded_hooks()
        ),
        Ok(Some(expected.to_owned()))
    );
}

#[rstest]
#[case::into_the_declared_provider(
    "my-copilot",
    "my-copilot",
    "github_copilot/gpt-4",
    "https://login-free.test:github_copilot"
)]
#[case::into_the_gemini_default("my-gemini", "my-gemini", "gemini/gemini-pro", GEMINI_PRO_GENERATE)]
#[case::only_for_an_exact_key("gemini/gemini-pro", "gemini", "openai/gpt-4o", GEMINI_PRO_GENERATE)]
fn alias_replaces_the_model_before_the_next_step(
    #[case] model: &str,
    #[case] alias_key: &str,
    #[case] alias_target: &str,
    #[case] expected: &str,
) {
    let alias_map = aliases(&[(alias_key, alias_target)]);
    assert_eq!(
        lookup(
            model,
            &ApiBaseParams::default(),
            &alias_map,
            &recorded_hooks()
        ),
        Ok(Some(expected.to_owned()))
    );
}

#[test]
fn alias_reaches_the_vertex_google_string() {
    let alias_map = aliases(&[("my-vertex", "vertex_ai/gemini-1.5-pro")]);
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: Some(String::from("us-central1")),
        vertex_project: Some(String::from("proj")),
        stream: false,
    };
    assert_eq!(
        lookup("my-vertex", &params, &alias_map, &recorded_hooks()),
        Ok(Some(String::from(
            "us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent"
        )))
    );
}

#[rstest]
#[case::from_the_custom_provider(
    "gpt-4",
    Some("github_copilot"),
    "https://login-free.test:github_copilot"
)]
#[case::from_the_model_prefix("chatgpt/gpt-4o", None, "https://login-free.test:chatgpt")]
#[case::the_custom_provider_beats_the_prefix(
    "chatgpt/gpt-4o",
    Some("github_copilot"),
    "https://login-free.test:github_copilot"
)]
fn declared_provider_returns_the_login_free_base(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected: &str,
) {
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: custom_llm_provider.map(str::to_owned),
        api_key: None,
        vertex_location: None,
        vertex_project: None,
        stream: false,
    };
    assert_eq!(
        lookup(model, &params, &BTreeMap::new(), &recorded_hooks()),
        Ok(Some(expected.to_owned()))
    );
}

#[test]
fn declared_login_free_none_stops_the_lookup() {
    let hooks = FakeHooks {
        login_free: None,
        claude: ClaudeHook::Records,
    };
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: Some(String::from("us-central1")),
        vertex_project: Some(String::from("proj")),
        stream: false,
    };
    assert_eq!(
        lookup("chatgpt/gemini-pro", &params, &BTreeMap::new(), &hooks),
        Ok(None)
    );
}

#[test]
fn resolution_dynamic_api_base_is_returned() {
    assert_eq!(
        lookup(
            "perplexity/llama-3-sonar",
            &ApiBaseParams::default(),
            &BTreeMap::new(),
            &recorded_hooks()
        ),
        Ok(Some(String::from("https://api.perplexity.ai")))
    );
}

#[test]
fn resolution_failure_skips_the_provider_defaults() {
    assert_eq!(
        lookup(
            "gemini-pro",
            &ApiBaseParams::default(),
            &BTreeMap::new(),
            &recorded_hooks()
        ),
        Ok(None)
    );
}

#[rstest]
#[case::keeps_the_given_model_for_the_vertex_branch(
    "unknown-provider/claude-x",
    "claude:us-central1:proj:unknown-provider/claude-x:false"
)]
#[case::builds_the_vertex_string_from_the_unresolved_model(
    "unknown-provider/gemini-pro",
    "us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/unknown-provider/gemini-pro:generateContent"
)]
#[case::still_returns_a_later_dynamic_base("perplexity/claude-x", "https://api.perplexity.ai")]
fn resolution_failure_and_the_vertex_branch(#[case] model: &str, #[case] expected: &str) {
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: Some(String::from("us-central1")),
        vertex_project: Some(String::from("proj")),
        stream: false,
    };
    assert_eq!(
        lookup(model, &params, &BTreeMap::new(), &recorded_hooks()),
        Ok(Some(expected.to_owned()))
    );
}

#[rstest]
#[case::google_without_stream(
    "vertex_ai/gemini-1.5-pro",
    Some("us-central1"),
    Some("proj"),
    false,
    "us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent"
)]
#[case::google_with_stream(
    "vertex_ai/gemini-1.5-pro",
    Some("us-central1"),
    Some("proj"),
    true,
    "us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:streamGenerateContent"
)]
#[case::location_without_project(
    "gemini/gemini-pro",
    Some("us-central1"),
    None,
    false,
    GEMINI_PRO_GENERATE
)]
#[case::project_without_location(
    "gemini/gemini-pro",
    None,
    Some("proj"),
    false,
    GEMINI_PRO_GENERATE
)]
#[case::the_location_is_not_validated(
    "vertex_ai/gemini-1.5-pro",
    Some("Bad.Location"),
    Some("proj"),
    false,
    "Bad.Location-aiplatform.googleapis.com/v1/projects/proj/locations/Bad.Location/publishers/google/models/gemini-1.5-pro:generateContent"
)]
#[case::an_empty_location_counts_as_set(
    "vertex_ai/gemini-1.5-pro",
    Some(""),
    Some("proj"),
    false,
    "-aiplatform.googleapis.com/v1/projects/proj/locations//publishers/google/models/gemini-1.5-pro:generateContent"
)]
fn vertex_google_url_branches(
    #[case] model: &str,
    #[case] location: Option<&str>,
    #[case] project: Option<&str>,
    #[case] stream: bool,
    #[case] expected: &str,
) {
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: location.map(str::to_owned),
        vertex_project: project.map(str::to_owned),
        stream,
    };
    assert_eq!(
        lookup(model, &params, &BTreeMap::new(), &recorded_hooks()),
        Ok(Some(expected.to_owned()))
    );
}

#[test]
fn vertex_claude_returns_the_hook_url() {
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: Some(String::from("us-central1")),
        vertex_project: Some(String::from("proj")),
        stream: true,
    };
    assert_eq!(
        lookup(
            "vertex_ai/claude-3-5-sonnet@20240620",
            &params,
            &BTreeMap::new(),
            &recorded_hooks()
        ),
        Ok(Some(String::from(
            "claude:us-central1:proj:claude-3-5-sonnet@20240620:true"
        )))
    );
}

#[test]
fn vertex_claude_hook_error_is_returned() {
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: Some(String::from("US-Central1")),
        vertex_project: Some(String::from("proj")),
        stream: false,
    };
    assert_eq!(
        lookup(
            "vertex_ai/claude-3-5-sonnet@20240620",
            &params,
            &BTreeMap::new(),
            &claude_error_hooks()
        ),
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::from("invalid vertex location US-Central1"),
        })
    );
}

#[rstest]
#[case::gemini_without_stream("gemini/gemini-pro", false, Some(GEMINI_PRO_GENERATE))]
#[case::gemini_with_stream(
    "gemini/gemini-pro",
    true,
    Some(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:streamGenerateContent"
    )
)]
#[case::openai("openai/gpt-4o", false, Some("https://api.openai.com"))]
#[case::another_provider("anthropic/claude-2", false, None)]
fn default_provider_bases(
    #[case] model: &str,
    #[case] stream: bool,
    #[case] expected: Option<&str>,
) {
    let params = ApiBaseParams {
        api_base: None,
        base_url: None,
        custom_llm_provider: None,
        api_key: None,
        vertex_location: None,
        vertex_project: None,
        stream,
    };
    assert_eq!(
        lookup(model, &params, &BTreeMap::new(), &recorded_hooks()),
        Ok(expected.map(str::to_owned))
    );
}
