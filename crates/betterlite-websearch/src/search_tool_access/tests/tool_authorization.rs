use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use crate::search_tool_access::authorization::{
    SearchToolAccessError, SearchToolAuthorizer, SearchToolGrants,
};
use crate::search_tool_access::hook_kwargs::{
    ParentOtelSpan, SearchCaller as Caller, SearchHookKwargs,
};
use crate::search_tool_access::tool_selection::SearchToolConfig;

type StepResult = Result<(), FakeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Key,
    Team,
    TeamCheck,
}

#[derive(Debug, PartialEq, Eq)]
struct Team {
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    CheckKey {
        name: String,
        team_id: Option<String>,
    },
    LoadTeam {
        team_id: String,
        has_span: bool,
    },
    CheckTeam {
        name: String,
        team_id: String,
    },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum FakeError {
    #[error("key check failed")]
    Key,
    #[error("team load failed")]
    Team,
    #[error("team check failed")]
    TeamCheck,
}

#[derive(Clone)]
struct Recorder {
    calls: Arc<Mutex<Vec<Call>>>,
    fail_at: Option<Step>,
}

impl Recorder {
    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    fn recorded(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn outcome(&self, step: Step, error: FakeError) -> Result<(), FakeError> {
        if self.fail_at == Some(step) {
            Err(error)
        } else {
            Ok(())
        }
    }
}

impl SearchToolGrants for Recorder {
    type Team = Team;
    type Error = FakeError;

    fn check_key(&self, name: &str, caller: &Caller) -> impl Future<Output = StepResult> + Send {
        self.record(Call::CheckKey {
            name: name.to_owned(),
            team_id: caller.team_id.clone(),
        });
        ready(self.outcome(Step::Key, FakeError::Key))
    }

    fn load_team(
        &self,
        team_id: &str,
        parent_otel_span: Option<&ParentOtelSpan>,
    ) -> impl Future<Output = Result<Team, FakeError>> + Send {
        self.record(Call::LoadTeam {
            team_id: team_id.to_owned(),
            has_span: parent_otel_span.is_some(),
        });
        let fails = self.fail_at == Some(Step::Team);
        let team = Team {
            id: team_id.to_owned(),
        };
        async move {
            if fails {
                Err(FakeError::Team)
            } else {
                Ok(team)
            }
        }
    }

    fn check_team(&self, name: &str, team: &Team) -> impl Future<Output = StepResult> + Send {
        self.record(Call::CheckTeam {
            name: name.to_owned(),
            team_id: team.id.clone(),
        });
        ready(self.outcome(Step::TeamCheck, FakeError::TeamCheck))
    }
}

fn make_recorder(fail_at: Option<Step>) -> Recorder {
    Recorder {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_at,
    }
}

fn tool(name: &str) -> SearchToolConfig {
    let json = format!(
        r#"{{"search_tool_name": "{name}", "litellm_params": {{"search_provider": "tavily"}}}}"#
    );
    serde_json::from_str(&json).unwrap()
}

fn kwargs_with_team(team_id: &str) -> SearchHookKwargs {
    let json = format!(r#"{{"metadata": {{"user_api_key_auth": {{"team_id": "{team_id}"}}}}}}"#);
    serde_json::from_str(&json).unwrap()
}

fn kwargs_with_team_and_span(team_id: &str) -> SearchHookKwargs {
    let json = format!(
        r#"{{"metadata": {{"user_api_key_auth": {{"team_id": "{team_id}", "parent_otel_span": "span"}}}}}}"#
    );
    serde_json::from_str(&json).unwrap()
}

fn kwargs_without_team() -> SearchHookKwargs {
    serde_json::from_str(r#"{"metadata": {"user_api_key_auth": {}}}"#).unwrap()
}

async fn authorize(
    recorder: &Recorder,
    tool: Option<&SearchToolConfig>,
    kwargs: Option<&SearchHookKwargs>,
) -> Result<(), SearchToolAccessError<FakeError>> {
    let authorizer = SearchToolAuthorizer::from(recorder.clone());
    authorizer.authorize(tool, kwargs).await
}

async fn assert_allowed(
    recorder: &Recorder,
    tool: Option<&SearchToolConfig>,
    kwargs: Option<&SearchHookKwargs>,
) {
    authorize(recorder, tool, kwargs).await.unwrap();
}

#[tokio::test]
async fn skips_when_the_tool_has_no_name() {
    let recorder = make_recorder(None);
    let unnamed = tool("");
    let kwargs = kwargs_with_team("t1");
    assert_allowed(&recorder, Some(&unnamed), Some(&kwargs)).await;
    assert!(recorder.recorded().is_empty());
}

#[tokio::test]
async fn skips_when_no_caller_is_found() {
    let recorder = make_recorder(None);
    let named = tool("search");
    assert_allowed(&recorder, Some(&named), None).await;
    let empty = SearchHookKwargs::default();
    assert_allowed(&recorder, Some(&named), Some(&empty)).await;
    assert!(recorder.recorded().is_empty());
}

#[tokio::test]
async fn checks_the_key_then_loads_and_checks_the_team() {
    let recorder = make_recorder(None);
    let named = tool("search");
    let kwargs = kwargs_with_team("t1");
    assert_allowed(&recorder, Some(&named), Some(&kwargs)).await;
    assert_eq!(
        recorder.recorded(),
        [
            Call::CheckKey {
                name: "search".to_owned(),
                team_id: Some("t1".to_owned())
            },
            Call::LoadTeam {
                team_id: "t1".to_owned(),
                has_span: false
            },
            Call::CheckTeam {
                name: "search".to_owned(),
                team_id: "t1".to_owned()
            }
        ]
    );
}

#[tokio::test]
async fn checks_only_the_key_without_a_team_id() {
    let recorder = make_recorder(None);
    let named = tool("search");
    let kwargs = kwargs_without_team();
    assert_allowed(&recorder, Some(&named), Some(&kwargs)).await;
    assert_eq!(
        recorder.recorded(),
        [Call::CheckKey {
            name: "search".to_owned(),
            team_id: None
        }]
    );
}

#[tokio::test]
async fn skips_the_team_steps_for_an_empty_team_id() {
    let recorder = make_recorder(None);
    let named = tool("search");
    let kwargs = kwargs_with_team("");
    assert_allowed(&recorder, Some(&named), Some(&kwargs)).await;
    assert_eq!(
        recorder.recorded(),
        [Call::CheckKey {
            name: "search".to_owned(),
            team_id: Some(String::new())
        }]
    );
}

#[tokio::test]
async fn passes_the_callers_parent_span_to_the_team_lookup() {
    let recorder = make_recorder(None);
    let named = tool("search");
    let kwargs = kwargs_with_team_and_span("t1");
    assert_allowed(&recorder, Some(&named), Some(&kwargs)).await;
    assert!(matches!(
        recorder.recorded().as_slice(),
        [_, Call::LoadTeam { has_span: true, .. }, _]
    ));
}

#[tokio::test]
async fn allows_an_absent_tool() {
    let recorder = make_recorder(None);
    let kwargs = kwargs_with_team("t1");
    assert_allowed(&recorder, None, Some(&kwargs)).await;
    assert!(recorder.recorded().is_empty());
}

#[tokio::test]
async fn returns_the_key_error_without_later_steps() {
    let recorder = make_recorder(Some(Step::Key));
    let named = tool("search");
    let kwargs = kwargs_with_team("t1");
    let error = authorize(&recorder, Some(&named), Some(&kwargs))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SearchToolAccessError::Grants(FakeError::Key)
    ));
    assert_eq!(
        recorder.recorded(),
        [Call::CheckKey {
            name: "search".to_owned(),
            team_id: Some("t1".to_owned())
        }]
    );
}

#[tokio::test]
async fn returns_the_team_lookup_error_without_the_team_check() {
    let recorder = make_recorder(Some(Step::Team));
    let named = tool("search");
    let kwargs = kwargs_with_team("t1");
    let error = authorize(&recorder, Some(&named), Some(&kwargs))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SearchToolAccessError::Grants(FakeError::Team)
    ));
    assert!(matches!(
        recorder.recorded().as_slice(),
        [Call::CheckKey { .. }, Call::LoadTeam { .. }]
    ));
}

#[tokio::test]
async fn returns_the_team_check_error_after_all_three_steps() {
    let recorder = make_recorder(Some(Step::TeamCheck));
    let named = tool("search");
    let kwargs = kwargs_with_team("t1");
    let error = authorize(&recorder, Some(&named), Some(&kwargs))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SearchToolAccessError::Grants(FakeError::TeamCheck)
    ));
    assert_eq!(recorder.recorded().len(), 3);
}
