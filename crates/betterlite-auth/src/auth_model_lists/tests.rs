#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;

use super::access_groups::{AccessGroupScope, models_from_access_groups};
use super::complete_list::{ListingFlags, ListingRequest, get_complete_model_list};
use super::credential::{CredentialEntry, CredentialSnapshot, hydrate_credential_name};
use super::fallbacks::{FallbackError, FallbackGroups, get_all_fallbacks};
use super::key_models::get_key_models;
use super::lists::NO_DEFAULT_MODELS;
use super::mcp::{ObjectPermissions, PermissionLookupError, get_mcp_server_ids};
use super::model_info::expand_wildcard_deployments;
use super::parameters::{ModelParameters, ParameterError};
use super::providers::{ProviderModels, get_provider_models, is_declared_provider};
use super::team_models::get_team_models;
use super::wildcard::{
    DeploymentLookup, ExpansionContext, WildcardError, get_known_models_from_wildcard,
};
use crate::auth_model_patterns::pattern::is_wildcard_pattern;
use crate::auth_model_patterns::{ALL_PROXY_MODELS, ALL_TEAM_MODELS};

fn strings(models: &[&str]) -> Vec<String> {
    models.iter().map(|model| (*model).to_owned()).collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn params(entries: &[(&str, Json)]) -> ModelParameters {
    ModelParameters::parse(&object(entries)).expect("the test parameters parse")
}

fn settings_with_groups() -> Settings {
    Settings {
        fallbacks: vec![group("general-key", "general-target")],
        context_window_fallbacks: vec![group("context-key", "context-target")],
        content_policy_fallbacks: vec![group("policy-key", "policy-target")],
        max_fallbacks: 5,
    }
}

fn group(name: &str, target: &str) -> FallbackChainEntry {
    FallbackChainEntry::Group(IndexMap::from([(
        name.to_owned(),
        vec![FallbackTarget::Model(target.to_owned())],
    )]))
}

#[derive(Clone, Debug)]
struct LookupRecord {
    provider: Option<String>,
    parameters: Option<ModelParameters>,
}

struct FakeProviders {
    environment: Vec<String>,
    by_provider: BTreeMap<String, Vec<String>>,
    records: Mutex<Vec<LookupRecord>>,
}

impl FakeProviders {
    fn new(environment: &[&str], by_provider: &[(&str, &[&str])]) -> Self {
        Self {
            environment: strings(environment),
            by_provider: by_provider
                .iter()
                .map(|(provider, models)| ((*provider).to_owned(), strings(models)))
                .collect(),
            records: Mutex::new(Vec::new()),
        }
    }

    fn records(&self) -> Vec<LookupRecord> {
        self.records
            .lock()
            .expect("the fake records its lookups")
            .clone()
    }
}

impl ProviderModels for FakeProviders {
    fn valid_models(
        &self,
        provider: Option<&str>,
        parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        self.records
            .lock()
            .expect("the fake records its lookups")
            .push(LookupRecord {
                provider: provider.map(str::to_owned),
                parameters: parameters.cloned(),
            });
        provider.map_or_else(
            || Some(self.environment.clone()),
            |name| self.by_provider.get(name).cloned(),
        )
    }
}

type RowSpec<'row> = (&'row str, Option<&'row str>, &'row [(&'row str, Json)]);

struct DeploymentRow {
    model: String,
    team_id: Option<String>,
    parameters: Vec<JsonObject>,
}

struct FakeDeployments {
    rows: Vec<DeploymentRow>,
    fails: bool,
}

impl FakeDeployments {
    fn rows(rows: &[RowSpec<'_>]) -> Self {
        Self {
            rows: rows
                .iter()
                .map(|(model, team_id, parameters)| DeploymentRow {
                    model: (*model).to_owned(),
                    team_id: team_id.map(str::to_owned),
                    parameters: vec![object(parameters)],
                })
                .collect(),
            fails: false,
        }
    }
}

impl DeploymentLookup for FakeDeployments {
    fn model_rows(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<Vec<ModelParameters>, WildcardError> {
        if self.fails {
            return Err(WildcardError::Lookup);
        }
        let mut parsed = Vec::new();
        for row in &self.rows {
            if row.model != model || row.team_id.as_deref() != team_id {
                continue;
            }
            for parameters in &row.parameters {
                parsed.push(ModelParameters::parse(parameters)?);
            }
        }
        Ok(parsed)
    }
}

struct FakeFallbacks {
    plans: BTreeMap<String, Vec<String>>,
    fails: bool,
    seen: Mutex<Vec<Vec<String>>>,
}

impl FakeFallbacks {
    fn new(plans: &[(&str, &[&str])]) -> Self {
        let mut by_model = BTreeMap::new();
        for (model, models) in plans {
            by_model.insert((*model).to_owned(), strings(models));
        }
        Self {
            plans: by_model,
            fails: false,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen_chains(&self) -> usize {
        self.seen.lock().expect("the fake records its chains").len()
    }

    // the last chain's own group names, so a test can tell which category reached the resolver
    fn seen_groups(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("the fake records its chains")
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

impl FallbackGroups for FakeFallbacks {
    fn fallback_group(
        &self,
        chain: &[FallbackChainEntry],
        model: &str,
    ) -> Result<Option<Vec<String>>, FallbackError> {
        let names: Vec<String> = chain
            .iter()
            .flat_map(|entry| match entry {
                FallbackChainEntry::Group(group) => group.keys().cloned().collect::<Vec<_>>(),
                FallbackChainEntry::Target(_) => Vec::new(),
            })
            .collect();
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(names);
        if self.fails {
            return Err(FallbackError::Unresolvable);
        }
        Ok(self.plans.get(model).cloned())
    }
}

struct FakePermissions {
    servers: Option<Vec<String>>,
    fails: bool,
}

#[async_trait::async_trait]
impl ObjectPermissions for FakePermissions {
    async fn mcp_servers(
        &self,
        _object_permission_id: &str,
    ) -> Result<Option<Vec<String>>, PermissionLookupError> {
        if self.fails {
            return Err(PermissionLookupError::Unreadable);
        }
        Ok(self.servers.clone())
    }
}

#[rstest]
#[case::star("*", true)]
#[case::provider_wide("openai/*", true)]
#[case::mid_word("gpt-*o", true)]
#[case::plain("gpt-4o", false)]
#[case::empty("", false)]
fn wildcard_any_asterisk_and_provider_lookup(#[case] model: &str, #[case] wildcard: bool) {
    assert_eq!(is_wildcard_pattern(model), wildcard);
    let providers = FakeProviders::new(
        &["environment-model"],
        &[("openai", &["gpt-4o"]), ("anthropic", &["claude"])],
    );
    let parameters = params(&[("model", text("anthropic/*"))]);
    assert_eq!(
        get_provider_models("*", Some(&parameters), &providers),
        Some(strings(&["environment-model"]))
    );
    assert_eq!(
        get_provider_models("openai", Some(&parameters), &providers),
        Some(strings(&["gpt-4o"]))
    );
    assert_eq!(
        get_provider_models("missing", Some(&parameters), &providers),
        None
    );
    let records = providers.records();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].provider, None);
    assert_eq!(records[1].provider.as_deref(), Some("openai"));
    assert_eq!(records[1].parameters.as_ref(), Some(&parameters));
}

#[test]
fn group_name_model_collision() {
    let groups = IndexMap::from([
        ("beta-models".to_owned(), strings(&["gpt-4", "claude"])),
        ("shared".to_owned(), strings(&["backing"])),
    ]);
    let all_models = strings(&["gpt-4", "shared", "beta-models"]);
    let proxy_model_list = strings(&["shared", "other"]);
    // a name that is both a grant and a deployed model stays; a mere group name is dropped
    assert_eq!(
        models_from_access_groups(&groups, &all_models, false, &proxy_model_list),
        strings(&["gpt-4", "shared", "backing", "gpt-4", "claude"])
    );
    assert_eq!(
        models_from_access_groups(&groups, &all_models, true, &proxy_model_list),
        strings(&[
            "gpt-4",
            "shared",
            "beta-models",
            "backing",
            "gpt-4",
            "claude"
        ])
    );
}

#[test]
fn key_sentinel_replacement_and_ordered_dedup() {
    let groups = IndexMap::from([("beta".to_owned(), strings(&["member"]))]);
    let proxy_model_list = strings(&["proxy-a", "proxy-b"]);
    let team_models = strings(&["team-a", "proxy-a"]);
    let scope = AccessGroupScope {
        include: false,
        only: false,
    };
    let key_models = strings(&[ALL_TEAM_MODELS]);
    assert_eq!(
        get_key_models(&key_models, &team_models, &proxy_model_list, &groups, scope),
        strings(&["team-a", "proxy-a"])
    );
    let nested_team_models = strings(&[ALL_TEAM_MODELS, "team-a"]);
    assert_eq!(
        get_key_models(
            &key_models,
            &nested_team_models,
            &proxy_model_list,
            &groups,
            scope
        ),
        strings(&["team-a", "proxy-a", "proxy-b"])
    );
    let repeated = strings(&["b", "a", "b"]);
    assert_eq!(
        get_key_models(&repeated, &team_models, &proxy_model_list, &groups, scope),
        strings(&["b", "a"])
    );
    assert!(get_key_models(&[], &team_models, &proxy_model_list, &groups, scope).is_empty());
}

#[test]
fn key_only_groups_flag_unused() {
    let groups = IndexMap::from([("beta".to_owned(), strings(&["member"]))]);
    let proxy_model_list = strings(&["proxy-a"]);
    let team_models = strings(&[ALL_TEAM_MODELS]);
    let key_models = strings(&[ALL_PROXY_MODELS]);
    let plain = AccessGroupScope {
        include: true,
        only: false,
    };
    let only = AccessGroupScope {
        include: true,
        only: true,
    };
    let expected = get_key_models(&key_models, &team_models, &proxy_model_list, &groups, plain);
    assert_eq!(
        expected,
        strings(&["proxy-a", "beta", "member"]),
        "the proxy sentinel expands and the group key is included"
    );
    assert_eq!(
        get_key_models(&key_models, &team_models, &proxy_model_list, &groups, only),
        expected,
        "only_model_access_groups never reaches get_key_models"
    );
}

#[test]
fn team_set_order_and_all_proxy_sentinel_retained() {
    let proxy_model_list = strings(&["a-model"]);
    let groups = IndexMap::new();
    let team_models = strings(&[ALL_TEAM_MODELS, ALL_PROXY_MODELS, "z-model"]);
    assert_eq!(
        get_team_models(&team_models, &proxy_model_list, &groups, false),
        strings(&["a-model", ALL_PROXY_MODELS, "z-model"]),
        "all-team-models leaves and all-proxy-models stays in the sorted set"
    );
    let ordered = strings(&["team-b", "team-a", "team-b"]);
    assert_eq!(
        get_team_models(&ordered, &proxy_model_list, &groups, false),
        strings(&["team-a", "team-b"])
    );
    assert!(get_team_models(&[], &proxy_model_list, &groups, false).is_empty());
}

fn expansion_context<'context>(
    providers: &'context FakeProviders,
    credentials: &'context CredentialSnapshot,
) -> ExpansionContext<'context> {
    ExpansionContext {
        providers,
        credentials,
        deployments: None,
    }
}

#[test]
fn complete_key_then_team_then_proxy_precedence() {
    let providers = FakeProviders::new(&["inferred"], &[]);
    let credentials = CredentialSnapshot::default();
    let groups = IndexMap::new();
    let proxy_model_list = strings(&["proxy-a"]);
    let context = expansion_context(&providers, &credentials);
    let key_models = strings(&["key-a"]);
    let team_models = strings(&["team-a"]);
    let key_request = ListingRequest {
        key_models: &key_models,
        team_models: &team_models,
        user_model: Some("user-a"),
        team_id: None,
        model_access_groups: &groups,
        flags: ListingFlags::default(),
    };
    assert_eq!(
        get_complete_model_list(&proxy_model_list, &key_request, &context)
            .expect("the listing builds"),
        strings(&["key-a"])
    );
    let team_request = ListingRequest {
        key_models: &[],
        ..key_request
    };
    assert_eq!(
        get_complete_model_list(&proxy_model_list, &team_request, &context)
            .expect("the listing builds"),
        strings(&["team-a"])
    );
    let proxy_request = ListingRequest {
        team_models: &[],
        ..team_request
    };
    assert_eq!(
        get_complete_model_list(&proxy_model_list, &proxy_request, &context)
            .expect("the listing builds"),
        strings(&["proxy-a", "user-a"])
    );
}

#[test]
fn no_default_sentinel_and_inferred_models() {
    let providers = FakeProviders::new(&["inferred-a", "inferred-b"], &[]);
    let credentials = CredentialSnapshot::default();
    let groups = IndexMap::new();
    let proxy_model_list = strings(&[NO_DEFAULT_MODELS, "proxy-a"]);
    let context = expansion_context(&providers, &credentials);
    let request = ListingRequest {
        key_models: &[],
        team_models: &[],
        user_model: Some("user-a"),
        team_id: None,
        model_access_groups: &groups,
        flags: ListingFlags {
            infer_model_from_keys: true,
            return_wildcard_routes: false,
            groups: AccessGroupScope::default(),
        },
    };
    assert_eq!(
        get_complete_model_list(&proxy_model_list, &request, &context).expect("the listing builds"),
        strings(&["proxy-a", "user-a", "inferred-a", "inferred-b"])
    );
    let key_models = strings(&["key-a"]);
    let granted = ListingRequest {
        key_models: &key_models,
        ..request
    };
    assert_eq!(
        get_complete_model_list(&proxy_model_list, &granted, &context).expect("the listing builds"),
        strings(&["key-a"]),
        "a granted key never falls back to inferred provider models"
    );
    let empty_user = ListingRequest {
        user_model: Some(""),
        ..request
    };
    assert_eq!(
        get_complete_model_list(&strings(&["proxy-a"]), &empty_user, &context)
            .expect("the listing builds"),
        strings(&["proxy-a", "inferred-a", "inferred-b"])
    );
}

#[test]
fn only_groups_returns_before_wildcards() {
    let providers = FakeProviders::new(&["environment"], &[("openai", &["gpt-4o"])]);
    let credentials = CredentialSnapshot::default();
    let groups = IndexMap::from([("beta".to_owned(), strings(&["member"]))]);
    let proxy_model_list = strings(&["openai/*", "beta"]);
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    let request = ListingRequest {
        key_models: &[],
        team_models: &[],
        user_model: None,
        team_id: None,
        model_access_groups: &groups,
        flags: ListingFlags {
            infer_model_from_keys: false,
            return_wildcard_routes: true,
            groups: AccessGroupScope {
                include: false,
                only: true,
            },
        },
    };
    assert_eq!(
        get_complete_model_list(&proxy_model_list, &request, &context).expect("the listing builds"),
        strings(&["beta"])
    );
    assert!(
        providers.records().is_empty(),
        "the wildcard is never expanded"
    );
}

#[test]
fn team_scoped_wildcard_lookup_and_duplicates() {
    let providers = FakeProviders::new(
        &[],
        &[
            ("openai", &["gpt-4o"]),
            ("anthropic", &["claude", "claude"]),
        ],
    );
    let credentials = CredentialSnapshot::default();
    let groups = IndexMap::new();
    let deployments = FakeDeployments::rows(&[(
        "openai/*",
        Some("team-1"),
        &[("model", text("anthropic/*"))],
    )]);
    let team_models = strings(&["openai/*"]);
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: Some(&deployments),
    };
    let request = ListingRequest {
        key_models: &[],
        team_models: &team_models,
        user_model: None,
        team_id: Some("team-1"),
        model_access_groups: &groups,
        flags: ListingFlags {
            infer_model_from_keys: false,
            return_wildcard_routes: true,
            groups: AccessGroupScope::default(),
        },
    };
    assert_eq!(
        get_complete_model_list(&strings(&[]), &request, &context).expect("the listing builds"),
        strings(&["openai/*", "openai/claude", "openai/claude"]),
        "the team row redirects the provider and the expansions are never deduplicated"
    );
    let team_less = ListingRequest {
        team_id: None,
        ..request
    };
    assert_eq!(
        get_complete_model_list(&strings(&[]), &team_less, &context).expect("the listing builds"),
        strings(&["openai/*", "openai/gpt-4o"]),
        "without the team the row is invisible and the wildcard prefix names the provider"
    );
}

#[test]
fn wildcard_missing_slash_and_star_param_redirect() {
    let providers = FakeProviders::new(&[], &[("openai", &["gpt-4o"])]);
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    assert!(get_known_models_from_wildcard("*", None, &context).is_empty());
    assert!(get_known_models_from_wildcard("gpt-4o", None, &context).is_empty());
    let redirect = params(&[("model", text("openai/*"))]);
    assert_eq!(
        get_known_models_from_wildcard("*", Some(&redirect), &context),
        strings(&["openai/gpt-4o"])
    );
    let concrete = params(&[("model", text("gpt-4o"))]);
    assert!(get_known_models_from_wildcard("*", Some(&concrete), &context).is_empty());
    let slashed_concrete = params(&[("model", text("openai/gpt-4o"))]);
    assert!(get_known_models_from_wildcard("*", Some(&slashed_concrete), &context).is_empty());
}

#[test]
fn partial_suffix_filter_or_prepend() {
    let providers = FakeProviders::new(
        &[],
        &[
            ("gemini", &["gemini-1.5-pro", "gemini-1.5-flash", "other"]),
            ("anthropic", &["claude"]),
        ],
    );
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    assert_eq!(
        get_known_models_from_wildcard("gemini/gemini-1.5-*", None, &context),
        strings(&["gemini-1.5-pro", "gemini-1.5-flash"]),
        "the models already start with the wildcard's prefix, so it is never stacked twice"
    );
    assert_eq!(
        get_known_models_from_wildcard("anthropic/claude-*", None, &context),
        strings(&["anthropic/claude-claude"]),
        "no model starts with the prefix, so every model is prepended with it"
    );
}

#[test]
fn known_provider_prefix_replaced_org_prefix_preserved() {
    let providers = FakeProviders::new(
        &[],
        &[(
            "ollama_server1",
            &["ollama/gemma3:1b", "meta-llama/Llama-3-8B"],
        )],
    );
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    assert_eq!(
        get_known_models_from_wildcard("ollama_server1/*", None, &context),
        strings(&[
            "ollama_server1/gemma3:1b",
            "ollama_server1/meta-llama/Llama-3-8B"
        ])
    );
    assert!(is_declared_provider("ollama"));
    assert!(!is_declared_provider("veniceai"));
    assert!(!is_declared_provider("ollama_server1"));
}

#[test]
fn credential_first_match_copy_and_null_only_fields() {
    let parameters = params(&[
        ("model", text("openai/*")),
        ("litellm_credential_name", text("shared")),
        ("api_key", Json::Null),
        ("api_base", text("")),
        ("region_name", text("eu")),
        ("unrelated", text("kept")),
        (
            "vertex_credentials",
            Json::Object(object(&[("type", text("service_account"))])),
        ),
    ]);
    let credentials = CredentialSnapshot::new(vec![
        CredentialEntry {
            credential_name: "shared".to_owned(),
            credential_values: object(&[
                ("api_key", text("first")),
                ("api_base", text("first-base")),
                ("vertex_project", text("project-first")),
                ("unrelated", text("overwritten")),
            ]),
        },
        CredentialEntry {
            credential_name: "shared".to_owned(),
            credential_values: object(&[("api_key", text("second"))]),
        },
    ]);
    let hydrated = hydrate_credential_name(Some(&parameters), &credentials)
        .expect("the parameters stay present");
    assert_eq!(hydrated.text("api_key"), Some("first"));
    assert_eq!(hydrated.text("api_base"), Some(""));
    assert_eq!(hydrated.text("region_name"), Some("eu"));
    assert_eq!(hydrated.text("vertex_project"), Some("project-first"));
    assert_eq!(hydrated.text("unrelated"), Some("kept"));
    assert_eq!(hydrated.credential_name(), None);
    assert_eq!(parameters.text("vertex_project"), None);
    assert_eq!(parameters.credential_name(), Some("shared"));
}

#[test]
fn credential_name_clear_and_no_mutation() {
    let credentials = CredentialSnapshot::new(vec![CredentialEntry {
        credential_name: "named".to_owned(),
        credential_values: object(&[("api_key", text("stored"))]),
    }]);
    let hit = params(&[
        ("model", text("openai/*")),
        ("litellm_credential_name", text("named")),
    ]);
    let hydrated = hydrate_credential_name(Some(&hit), &credentials).expect("present");
    assert_eq!(hydrated.credential_name(), None);
    assert_eq!(hydrated.text("api_key"), Some("stored"));
    assert_eq!(
        credentials.values("named").get("api_key"),
        Some(&text("stored")),
        "hydrating a copy never rewrites the stored values"
    );
    let missing = params(&[
        ("model", text("openai/*")),
        ("litellm_credential_name", text("absent")),
    ]);
    let unchanged = hydrate_credential_name(Some(&missing), &credentials).expect("present");
    assert_eq!(unchanged, missing);
    let empty = CredentialSnapshot::new(vec![CredentialEntry {
        credential_name: "empty".to_owned(),
        credential_values: JsonObject::new(),
    }]);
    let empty_values = params(&[
        ("model", text("openai/*")),
        ("litellm_credential_name", text("empty")),
    ]);
    assert_eq!(
        hydrate_credential_name(Some(&empty_values), &empty),
        Some(empty_values.clone())
    );
    assert_eq!(hydrate_credential_name(None, &credentials), None);
}

#[test]
fn concrete_deployment_kept() {
    let providers = FakeProviders::new(&[], &[("openai", &["gpt-4o"])]);
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    let deployments = vec![object(&[
        ("model_name", text("gpt-4o")),
        (
            "litellm_params",
            Json::Object(object(&[("model", text("openai/*"))])),
        ),
    ])];
    assert_eq!(
        expand_wildcard_deployments(&deployments, &context),
        deployments
    );
    assert!(
        providers.records().is_empty(),
        "a concrete name is never expanded"
    );
}

#[test]
fn invalid_params_or_empty_expansion_keeps_row() {
    let providers = FakeProviders::new(&[], &[("known", &["model-a"])]);
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    let deployments = vec![
        object(&[
            ("model_name", text("openai/*")),
            (
                "litellm_params",
                Json::Object(object(&[("model", Json::Bool(true))])),
            ),
        ]),
        object(&[
            ("model_name", text("missing/*")),
            (
                "litellm_params",
                Json::Object(object(&[("model", text("missing/*"))])),
            ),
        ]),
        object(&[
            ("model_name", text("*")),
            ("litellm_params", text("not-an-object")),
        ]),
        object(&[("model_name", text("*"))]),
    ];
    assert_eq!(
        expand_wildcard_deployments(&deployments, &context),
        deployments
    );
    assert_eq!(
        providers.records().len(),
        1,
        "only the row naming a provider reaches the lookup; a wildcard with no slash returns first"
    );
}

#[test]
fn expanded_rows_deep_copy_and_params_model() {
    let providers = FakeProviders::new(&[], &[("openai", &["gpt-4o", "gpt-4o-mini"])]);
    let credentials = CredentialSnapshot::default();
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: None,
    };
    let deployment = object(&[
        ("model_name", text("openai/*")),
        (
            "litellm_params",
            Json::Object(object(&[
                ("model", text("openai/*")),
                ("api_key", text("key")),
            ])),
        ),
        (
            "model_info",
            Json::Object(object(&[("id", text("deployment-id"))])),
        ),
    ]);
    let expanded = expand_wildcard_deployments(std::slice::from_ref(&deployment), &context);
    assert_eq!(expanded.len(), 2);
    assert_expanded_row(&expanded[0], "openai/gpt-4o");
    assert_expanded_row(&expanded[1], "openai/gpt-4o-mini");
    assert_eq!(
        deployment.get("model_name"),
        Some(&text("openai/*")),
        "the input row is never mutated"
    );
}

fn assert_expanded_row(row: &JsonObject, model: &str) {
    assert_eq!(row.get("model_name"), Some(&text(model)));
    assert_eq!(
        row.get("model_info"),
        Some(&Json::Object(object(&[("id", text("deployment-id"))])))
    );
    let Some(Json::Object(parameters)) = row.get("litellm_params") else {
        panic!("every expanded row keeps its params object");
    };
    assert_eq!(parameters.get("api_key"), Some(&text("key")));
    assert_eq!(parameters.get("model"), Some(&text(model)));
}

#[test]
fn wildcard_lookup_error_propagates() {
    let providers = FakeProviders::new(&[], &[("openai", &["gpt-4o"])]);
    let credentials = CredentialSnapshot::default();
    let groups = IndexMap::new();
    let proxy_model_list = strings(&["openai/*"]);
    let request = ListingRequest {
        key_models: &[],
        team_models: &[],
        user_model: None,
        team_id: None,
        model_access_groups: &groups,
        flags: ListingFlags::default(),
    };
    let failing = FakeDeployments {
        rows: Vec::new(),
        fails: true,
    };
    let context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: Some(&failing),
    };
    assert!(matches!(
        get_complete_model_list(&proxy_model_list, &request, &context),
        Err(WildcardError::Lookup)
    ));
    let invalid = FakeDeployments::rows(&[("openai/*", None, &[("model", Json::Bool(true))])]);
    let invalid_context = ExpansionContext {
        providers: &providers,
        credentials: &credentials,
        deployments: Some(&invalid),
    };
    assert!(matches!(
        get_complete_model_list(&proxy_model_list, &request, &invalid_context),
        Err(WildcardError::Parameters(ParameterError::Model))
    ));
}

#[tokio::test]
async fn mcp_missing_context_and_query_error_silently_empty() {
    assert!(
        get_mcp_server_ids(None, Some("permission"))
            .await
            .is_empty()
    );
    let rowless = FakePermissions {
        servers: Some(strings(&["server-a", "server-b"])),
        fails: false,
    };
    assert!(get_mcp_server_ids(Some(&rowless), None).await.is_empty());
    let absent = FakePermissions {
        servers: None,
        fails: false,
    };
    assert!(
        get_mcp_server_ids(Some(&absent), Some("permission"))
            .await
            .is_empty()
    );
    let failing = FakePermissions {
        servers: Some(strings(&["server-a"])),
        fails: true,
    };
    assert!(
        get_mcp_server_ids(Some(&failing), Some("permission"))
            .await
            .is_empty()
    );
    assert_eq!(
        get_mcp_server_ids(Some(&rowless), Some("permission")).await,
        strings(&["server-a", "server-b"])
    );
}

#[test]
fn fallback_three_categories() {
    let settings = settings_with_groups();
    let resolver = FakeFallbacks::new(&[]);
    assert!(get_all_fallbacks("any-model", "general", Some(&settings), &resolver).is_empty());
    assert_eq!(resolver.seen_groups(), strings(&["general-key"]));
    assert!(
        get_all_fallbacks("any-model", "context_window", Some(&settings), &resolver).is_empty()
    );
    assert_eq!(resolver.seen_groups(), strings(&["context-key"]));
    assert!(
        get_all_fallbacks("any-model", "content_policy", Some(&settings), &resolver).is_empty()
    );
    assert_eq!(resolver.seen_groups(), strings(&["policy-key"]));
    let empty = Settings::default();
    assert!(get_all_fallbacks("any-model", "general", Some(&empty), &resolver).is_empty());
    assert!(get_all_fallbacks("any-model", "general", None, &resolver).is_empty());
    assert_eq!(
        resolver.seen_chains(),
        3,
        "an empty or absent list never reaches the plans"
    );
    let planned = FakeFallbacks::new(&[("gpt-4o", &["claude", "gemini"])]);
    assert_eq!(
        get_all_fallbacks("gpt-4o", "general", Some(&settings), &planned),
        strings(&["claude", "gemini"])
    );
    assert!(get_all_fallbacks("unknown", "general", Some(&settings), &planned).is_empty());
}

#[test]
fn unknown_fallback_warns_only_with_router() {
    let settings = settings_with_groups();
    let resolver = FakeFallbacks::new(&[]);
    let (value, warnings, errors) =
        events::captured(|| get_all_fallbacks("any-model", "bogus", Some(&settings), &resolver));
    assert!(value.is_empty());
    assert_eq!((warnings, errors), (1, 0));
    assert_eq!(resolver.seen_chains(), 0, "an unknown type never resolves");
    let (unrouted_value, unrouted_warnings, unrouted_errors) =
        events::captured(|| get_all_fallbacks("any-model", "bogus", None, &resolver));
    assert!(unrouted_value.is_empty());
    assert_eq!((unrouted_warnings, unrouted_errors), (0, 0));
    assert_eq!(
        resolver.seen_chains(),
        0,
        "no router never reaches the plans"
    );
}

#[test]
fn fallback_resolver_error_logs_and_returns_empty() {
    let settings = settings_with_groups();
    let failing = FakeFallbacks {
        plans: BTreeMap::new(),
        fails: true,
        seen: Mutex::new(Vec::new()),
    };
    let (value, warnings, errors) =
        events::captured(|| get_all_fallbacks("any-model", "general", Some(&settings), &failing));
    assert!(value.is_empty());
    assert_eq!((warnings, errors), (0, 1));
    assert_eq!(failing.seen_chains(), 1);
}

mod events;
mod shapes;
