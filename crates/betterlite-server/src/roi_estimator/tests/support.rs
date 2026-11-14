#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_roi::estimator::models::ROICompletionRequest;
use betterlite_roi::records::{ROIEstimate, ROIPullCommit, ROIPullEvidence, ROIPullFile};
use betterlite_roi::settings::ROISettings;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;

use crate::roi_estimator::{CompletionCaller, CompletionFuture, CompletionObject};
use crate::roi_github::error::SourceError;

pub(super) const OK_CONTENT: &str = r#"{"hours": 3.5, "reasoning": "Small change."}"#;
pub(super) const EVIDENCE: &str = r#"{"repo": "octocat/hello", "number": 12, "title": "Add the thing", "body": "Line one\nline two", "changes": {"additions": 10, "deletions": 2, "files": 3, "commits": 2}, "files": [{"filename": "a.py", "status": "added", "additions": 10, "deletions": 0}, {"filename": null, "status": null, "additions": null, "deletions": null}], "commits": [{"sha": "abc", "message": "first", "additions": 5}, {"sha": "def", "message": "second", "changed_files": null}]}"#;

#[derive(Serialize)]
struct CompletionFixture<'wire> {
    choices: Vec<ChoiceFixture<'wire>>,
}

#[derive(Serialize)]
struct ChoiceFixture<'wire> {
    finish_reason: Option<&'wire str>,
    message: MessageFixture<'wire>,
}

#[derive(Serialize)]
struct MessageFixture<'wire> {
    content: Option<&'wire str>,
}

pub(super) struct ModelSupport {
    pub supported: Vec<String>,
    pub seen: Vec<(String, String, Option<String>)>,
}

impl CapabilityPredicates for ModelSupport {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.seen.push((
            key.to_owned(),
            request.model.to_owned(),
            request.custom_llm_provider.map(str::to_owned),
        ));
        let supported = key == NamedCapability::NoneReasoningEffort.key()
            && self.supported.iter().any(|model| model == request.model);
        ready(supported)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

impl ModelSupport {
    pub(super) fn none() -> Self {
        Self {
            supported: Vec::new(),
            seen: Vec::new(),
        }
    }

    pub(super) fn supporting(models: &[&str]) -> Self {
        Self {
            supported: models.iter().map(|model| (*model).to_owned()).collect(),
            seen: Vec::new(),
        }
    }
}

pub(super) fn settings(model: &str, prompt: &str) -> ROISettings {
    ROISettings {
        github_api_url: "https://api.github.com".to_owned(),
        github_token: SecretString::default(),
        estimator_key: SecretString::default(),
        repos: Vec::new(),
        estimator_model: model.to_owned(),
        estimator_prompt: prompt.to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0_f64,
        identity_map: IndexMap::new(),
    }
}

pub(super) fn assert_hours(estimate: &ROIEstimate, expected: f64) {
    let hours = estimate.hours.expect("the estimate carries hours");
    assert_eq!(hours.to_bits(), expected.to_bits());
}

pub(super) fn completion(raw_json: &str) -> CompletionObject {
    serde_json::from_str(raw_json).unwrap()
}

pub(super) fn envelope(finish_reason: Option<&str>, content: Option<&str>) -> CompletionObject {
    let wire = CompletionFixture {
        choices: vec![ChoiceFixture {
            finish_reason,
            message: MessageFixture { content },
        }],
    };
    completion(&serde_json::to_string(&wire).unwrap())
}

pub(super) fn ok_response(content: &str) -> CompletionObject {
    envelope(Some("stop"), Some(content))
}

pub(super) fn pull() -> ROIPullEvidence {
    ROIPullEvidence {
        repo: "octocat/hello".to_owned(),
        number: 12,
        title: "Add the thing".to_owned(),
        body: "Line one\nline two".to_owned(),
        url: "https://github.com/octocat/hello/pull/12".to_owned(),
        login: "octocat".to_owned(),
        emails: Vec::new(),
        profile_email: String::new(),
        commit_emails: None,
        merged_at: "2026-01-01T00:00:00Z".to_owned(),
        head_sha: "abc123".to_owned(),
        additions: 10,
        deletions: 2,
        changed_files: 3,
        files: vec![
            ROIPullFile {
                filename: Some("a.py".to_owned()),
                status: Some("added".to_owned()),
                additions: Some(10),
                deletions: Some(0),
            },
            ROIPullFile {
                filename: None,
                status: None,
                additions: None,
                deletions: None,
            },
        ],
        commits: vec![
            ROIPullCommit {
                sha: "abc".to_owned(),
                message: "first".to_owned(),
                additions: Some(5),
                deletions: None,
                changed_files: None,
            },
            ROIPullCommit {
                sha: "def".to_owned(),
                message: "second".to_owned(),
                additions: None,
                deletions: None,
                changed_files: Some(None),
            },
        ],
        commit_count: 2,
        incomplete_metadata: false,
    }
}

pub(super) fn caller(
    response: Result<CompletionObject, SourceError>,
) -> (CompletionCaller, Arc<Mutex<Vec<ROICompletionRequest>>>) {
    let response = Arc::new(Mutex::new(Some(response)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let complete: CompletionCaller = Arc::new(move |request: ROICompletionRequest| {
        seen.lock().unwrap().push(request);
        let reply = response
            .lock()
            .unwrap()
            .take()
            .expect("the scripted completion is asked once");
        let future: CompletionFuture = Box::pin(ready(reply));
        future
    });
    (complete, requests)
}

pub(super) fn recorded_request(
    requests: &Arc<Mutex<Vec<ROICompletionRequest>>>,
) -> ROICompletionRequest {
    requests.lock().unwrap().first().unwrap().clone()
}
