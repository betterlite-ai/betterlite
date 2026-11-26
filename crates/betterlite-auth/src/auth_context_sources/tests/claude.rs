use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::auth_context_sources::claude::{
    AliasMap, CLAUDE_MODEL_ROUTES, ClaudeNameSources, ClaudeNormalization,
    ClaudeNormalizationInput, RouterNames, claude_code_requested_group, normalize_claude_model,
};
use crate::auth_context_sources::context::ModelContext;
use crate::auth_request_models::catalog::{Collaborators, EmptyCatalog};
use crate::auth_request_models::sources::DispatchContext;

use super::support::{FakeRouter, FakeTeams, NoDecoders, TeamAnswer, json, object, set_of};

const GROUP: &str = "group-one";

fn encoded(name: &str) -> String {
    format!("claude-router-{}", hex::encode(name))
}

struct ClaudeFixture {
    route: String,
    body: JsonObject,
    global_aliases: BTreeMap<String, String>,
    key_aliases: Option<Json>,
    team_model_aliases: Option<Json>,
    key_router_settings: Option<Json>,
    normalized: bool,
    teams: FakeTeams,
}

impl ClaudeFixture {
    fn new(model: &str) -> Self {
        Self::from_body(&format!(r#"{{"model": "{model}"}}"#))
    }

    fn from_body(body: &str) -> Self {
        Self {
            route: "/v1/chat/completions".to_owned(),
            body: object(body),
            global_aliases: BTreeMap::new(),
            key_aliases: None,
            team_model_aliases: None,
            key_router_settings: None,
            normalized: false,
            teams: FakeTeams::new(TeamAnswer::Absent),
        }
    }

    #[must_use]
    fn route(mut self, route: &str) -> Self {
        self.route = route.to_owned();
        self
    }

    #[must_use]
    fn global_alias(mut self, name: &str) -> Self {
        self.global_aliases
            .insert(name.to_owned(), "target".to_owned());
        self
    }

    #[must_use]
    fn key_aliases(mut self, text: &str) -> Self {
        self.key_aliases = Some(json(text));
        self
    }

    #[must_use]
    fn team_model_aliases(mut self, text: &str) -> Self {
        self.team_model_aliases = Some(json(text));
        self
    }

    #[must_use]
    fn key_router_settings(mut self, text: &str) -> Self {
        self.key_router_settings = Some(json(text));
        self
    }

    #[must_use]
    fn normalized(mut self) -> Self {
        self.normalized = true;
        self
    }

    async fn run(&self, router: Option<&dyn RouterNames>) -> ClaudeNormalization {
        let path_params = BTreeSet::new();
        let decoders = NoDecoders;
        let catalog = EmptyCatalog;
        let collaborators = Collaborators {
            catalog: Some(&catalog),
            decoders: &decoders,
        };
        let model_context = ModelContext {
            route: &self.route,
            body: &self.body,
            headers: None,
            query: None,
            dispatch: DispatchContext {
                endpoint_marker: None,
                path_params: &path_params,
            },
            team_id: None,
        };
        let input = ClaudeNormalizationInput {
            route: &self.route,
            request_data: &self.body,
            model_context,
            collaborators: &collaborators,
            router,
            team_id: None,
            global_aliases: &self.global_aliases,
            key_aliases: self.key_aliases.as_ref().map(AliasMap::Json),
            team_model_aliases: self.team_model_aliases.as_ref().map(AliasMap::Json),
            key_router_settings: self.key_router_settings.as_ref(),
            normalized: self.normalized,
        };
        normalize_claude_model(&input, &self.teams)
            .await
            .expect("the catalog answers")
    }
}

#[test]
fn alias_normalization() {
    let global_aliases = BTreeMap::from([(GROUP.to_owned(), "target".to_owned())]);
    let payload = Payload(BTreeMap::from([(
        "payload-alias".to_owned(),
        JsonData::Null,
    )]));
    let router = FakeRouter::default();
    let names = ClaudeNameSources {
        global_aliases: &global_aliases,
        key_aliases: Some(AliasMap::Payload(&payload)),
        team_model_aliases: None,
        hierarchical_model_group_alias: None,
        router: Some(&router),
        team_id: Some("team-one"),
    };
    assert_eq!(claude_code_requested_group(GROUP, &names), None);
    assert_eq!(
        claude_code_requested_group(&encoded(GROUP), &names),
        Some(GROUP.to_owned())
    );
    assert_eq!(
        claude_code_requested_group("group-one[1m]", &names),
        Some(GROUP.to_owned())
    );
    assert_eq!(
        claude_code_requested_group("group-one[1M]", &names),
        Some(GROUP.to_owned())
    );
    // the key's alias payload answers the same question for the decoded id
    assert_eq!(
        claude_code_requested_group(&encoded("payload-alias"), &names),
        Some("payload-alias".to_owned())
    );
    // a malformed or non-canonical compatibility id decodes to nothing
    assert_eq!(
        claude_code_requested_group("claude-router-zz", &names),
        None
    );
    let uppercase = format!("claude-router-{}", hex::encode(GROUP).to_uppercase());
    assert_eq!(claude_code_requested_group(&uppercase, &names), None);
}

#[test]
fn alias_canonical_decoding_only() {
    let global_aliases = BTreeMap::new();
    let router = FakeRouter::default();
    let names = ClaudeNameSources {
        global_aliases: &global_aliases,
        key_aliases: None,
        team_model_aliases: None,
        hierarchical_model_group_alias: None,
        router: Some(&router),
        team_id: None,
    };
    // hex of "group-one[1m]" decodes to a name no route owns
    assert_eq!(
        claude_code_requested_group(&encoded("group-one[1m]"), &names),
        None
    );
    assert_eq!(claude_code_requested_group("[1m]", &names), None);
    assert_eq!(claude_code_requested_group("unknown-group", &names), None);
}

#[rstest]
#[case::messages("/messages", true)]
#[case::v1_messages("/v1/messages", true)]
#[case::chat_completions("/chat/completions", true)]
#[case::v1_chat_completions("/v1/chat/completions", true)]
#[case::responses("/responses", true)]
#[case::v1_responses("/v1/responses", true)]
#[case::embeddings("/v1/embeddings", false)]
#[case::models("/v1/models", false)]
#[case::messages_trailing_slash("/messages/", false)]
#[case::v1_messages_trailing_slash("/v1/messages/", false)]
#[tokio::test]
async fn claude_six_routes_only(#[case] route: &str, #[case] eligible: bool) {
    assert_eq!(CLAUDE_MODEL_ROUTES.contains(&route), eligible, "{route}");
    let router = FakeRouter::new(BTreeSet::new(), set_of(&[GROUP]), BTreeMap::new());
    let outcome = ClaudeFixture::new(&encoded(GROUP))
        .route(route)
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: eligible,
            source: eligible.then(|| GROUP.to_owned()),
        },
        "{route}"
    );
    assert_eq!(router.asked().is_empty(), !eligible, "{route}");
}

#[tokio::test]
async fn claude_marker_skip() {
    let router = FakeRouter::new(BTreeSet::new(), set_of(&[GROUP]), BTreeMap::new());
    let outcome = ClaudeFixture::new(&encoded(GROUP))
        .normalized()
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: None,
        }
    );
    assert!(router.asked().is_empty());
}

#[tokio::test]
async fn claude_body_identity() {
    let router = FakeRouter::new(BTreeSet::new(), set_of(&[GROUP]), BTreeMap::new());
    let listed = format!(r#"{{"model": ["{0}", "{0}"]}}"#, encoded(GROUP));
    let outcome = ClaudeFixture::from_body(&listed).run(Some(&router)).await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: false,
            source: None,
        }
    );
    // the routed model comes from target_model_names while body.model stays empty
    let routed = format!(
        r#"{{"model": "", "target_model_names": "{}"}}"#,
        encoded(GROUP)
    );
    let outcome = ClaudeFixture::from_body(&routed).run(Some(&router)).await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: false,
            source: None,
        }
    );
    let outcome = ClaudeFixture::from_body(r#"{"model": 7}"#)
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: false,
            source: None,
        }
    );
}

#[rstest]
#[case::lowercase_suffix("group-one[1m]", true, Some(GROUP))]
#[case::uppercase_suffix("group-one[1M]", true, Some(GROUP))]
#[case::text_after_suffix("group-one[1m]x", false, None)]
#[case::uppercase_prefix("Claude-Router-666f6f", false, None)]
#[case::prefix_before_name("[1m]group-one", false, None)]
#[case::space_in_suffix("group-one[1 m]", false, None)]
#[case::unconfigured_suffix("unconfigured[1m]", true, None)]
#[tokio::test]
async fn claude_prefix_suffix_case(
    #[case] model: &str,
    #[case] normalized: bool,
    #[case] source: Option<&str>,
) {
    let router = FakeRouter::new(BTreeSet::new(), set_of(&[GROUP]), BTreeMap::new());
    let outcome = ClaudeFixture::new(model).run(Some(&router)).await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized,
            source: source.map(str::to_owned),
        },
        "{model}"
    );
}

#[tokio::test]
async fn claude_marks_unresolved() {
    let router = FakeRouter::default();
    let outcome = ClaudeFixture::new(&encoded("unknown-group"))
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: None,
        }
    );
    assert!(!router.asked().is_empty());
}

async fn assert_router_names_group(router: FakeRouter) {
    let outcome = ClaudeFixture::new(&encoded(GROUP)).run(Some(&router)).await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: Some(GROUP.to_owned()),
        }
    );
    assert!(!router.asked().is_empty());
}

#[tokio::test]
async fn claude_alias_order() {
    let requested = encoded(GROUP);
    // a route that owns the view id itself needs no rewrite and never reaches the router
    let router = FakeRouter::new(BTreeSet::new(), set_of(&[GROUP]), BTreeMap::new());
    let mut fixture = ClaudeFixture::new(&requested);
    fixture
        .global_aliases
        .insert(requested.clone(), "target".to_owned());
    let outcome = fixture.run(Some(&router)).await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: None,
        }
    );
    assert!(router.asked().is_empty());

    // the global alias map answers the decoded id
    let outcome = ClaudeFixture::new(&requested)
        .global_alias(GROUP)
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: Some(GROUP.to_owned()),
        }
    );

    // the key aliases come next
    let outcome = ClaudeFixture::new(&requested)
        .key_aliases(&format!(r#"{{"{GROUP}": "target"}}"#))
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: Some(GROUP.to_owned()),
        }
    );

    // then the team's aliases
    let outcome = ClaudeFixture::new(&requested)
        .team_model_aliases(&format!(r#"{{"{GROUP}": "target"}}"#))
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: Some(GROUP.to_owned()),
        }
    );

    // then the hierarchical router settings' model_group_alias
    let outcome = ClaudeFixture::new(&requested)
        .key_router_settings(&format!(
            r#"{{"model_group_alias": {{"{GROUP}": "target"}}}}"#
        ))
        .run(Some(&router))
        .await;
    assert_eq!(
        outcome,
        ClaudeNormalization {
            normalized: true,
            source: Some(GROUP.to_owned()),
        }
    );

    // and finally the router's own model group aliases, model ids and candidate ids
    assert_router_names_group(FakeRouter::new(
        set_of(&[GROUP]),
        BTreeSet::new(),
        BTreeMap::new(),
    ))
    .await;
    assert_router_names_group(FakeRouter::new(
        BTreeSet::new(),
        set_of(&[GROUP]),
        BTreeMap::new(),
    ))
    .await;
    assert_router_names_group(FakeRouter::new(
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeMap::from([(GROUP.to_owned(), vec!["deployment-id".to_owned()])]),
    ))
    .await;
}
