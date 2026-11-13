#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_roi::estimator::models::ROICompletionRequest;
use betterlite_roi::records::{ROIPullCommit, ROIPullEvidence, ROIPullFile};
use betterlite_roi::settings::ROISettings;
use serde_json::{Value, json};

use crate::roi_estimator::{CompletionCaller, CompletionFuture};
use crate::roi_github::error::SourceError;

pub const OK_CONTENT: &str = r#"{"hours": 3.5, "reasoning": "Small change."}"#;
pub const EVIDENCE: &str = r#"{"repo": "octocat/hello", "number": 12, "title": "Add the thing", "body": "Line one\nline two", "changes": {"additions": 10, "deletions": 2, "files": 3, "commits": 2}, "files": [{"filename": "a.py", "status": "added", "additions": 10, "deletions": 0}, {"filename": null, "status": null, "additions": null, "deletions": null}], "commits": [{"sha": "abc", "message": "first", "additions": 5}, {"sha": "def", "message": "second", "changed_files": null}]}"#;

pub struct ModelSupport {
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
    pub fn none() -> Self {
        Self {
            supported: Vec::new(),
            seen: Vec::new(),
        }
    }

    pub fn supporting(models: &[&str]) -> Self {
        Self {
            supported: models.iter().map(|model| (*model).to_owned()).collect(),
            seen: Vec::new(),
        }
    }
}

pub fn settings(model: &str, prompt: &str) -> ROISettings {
    let mut settings = ROISettings::default();
    settings.estimator_model = model.to_owned();
    settings.estimator_prompt = prompt.to_owned();
    settings
}

pub fn pull() -> ROIPullEvidence {
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

pub fn ok_response(content: &str) -> Value {
    json!({"choices": [{"finish_reason": "stop", "message": {"content": content}}]})
}

pub fn caller(
    response: Result<Value, SourceError>,
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

pub fn recorded_request(requests: &Arc<Mutex<Vec<ROICompletionRequest>>>) -> ROICompletionRequest {
    requests.lock().unwrap().first().unwrap().clone()
}
