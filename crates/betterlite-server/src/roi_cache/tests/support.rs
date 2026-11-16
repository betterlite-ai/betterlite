#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_roi::records::{
    ROIEstimate, ROIEstimateStatus, ROIPullEvidence, ROIPullRecord, ROIReport,
};
use betterlite_roi::settings::ROISettings;
use chrono::NaiveDate;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::roi_cache::processed::ProcessedPull;
use crate::roi_cache::store::ReportRepository;
use crate::roi_estimator::{CompletionCaller, CompletionFuture, CompletionObject};
use crate::roi_github::client::GitHub;
use crate::roi_github::error::SourceError;
use crate::roi_github::models::{GitHubHead, GitHubPullListItem, GitHubUser};
use crate::roi_github::transport::Transport;

pub(super) const SETTINGS_URL: &str = "https://api.github.com";

#[derive(Clone, Default)]
pub(super) struct Routes {
    answers: Arc<Mutex<BTreeMap<String, (u16, String)>>>,
    requested: Arc<Mutex<Vec<String>>>,
}

impl Routes {
    pub(super) fn with(self, path: &str, status: u16, body: &str) -> Self {
        self.answers
            .lock()
            .unwrap()
            .insert(path.to_owned(), (status, body.to_owned()));
        self
    }

    pub(super) fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl Transport for Routes {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let path = request.url.path().to_owned();
        self.requested.lock().unwrap().push(path.clone());
        let (status, body) = self
            .answers
            .lock()
            .unwrap()
            .get(&path)
            .cloned()
            .unwrap_or((404, String::new()));
        Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        })
    }
}

pub(super) fn settings() -> ROISettings {
    ROISettings {
        github_api_url: SETTINGS_URL.to_owned(),
        github_token: SecretString::default(),
        estimator_key: SecretString::default(),
        repos: vec!["a/b".to_owned(), "c/d".to_owned()],
        estimator_model: "gpt-5.2".to_owned(),
        estimator_prompt: "Estimate this.".to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0_f64,
        identity_map: IndexMap::new(),
    }
}

pub(super) fn github(routes: Routes) -> GitHub<Routes> {
    GitHub::new(&settings(), routes).unwrap()
}

pub(super) fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

pub(super) fn pull_item(number: i64, login: Option<&str>, sha: Option<&str>) -> GitHubPullListItem {
    GitHubPullListItem {
        number,
        html_url: format!("https://github.com/o/r/pull/{number}"),
        merged_at: Some("2026-01-05T00:00:00Z".to_owned()),
        updated_at: "2026-01-06T00:00:00Z".to_owned(),
        title: format!("Pull {number}"),
        body: Some("Body".to_owned()),
        head: sha.map(|sha| GitHubHead {
            sha: sha.to_owned(),
        }),
        user: login.map(|login| GitHubUser {
            login: Some(login.to_owned()),
        }),
    }
}

pub(super) fn pulls_merged_on(merged: &[(i64, &str)]) -> String {
    let items: Vec<String> = merged
        .iter()
        .map(|(number, day)| {
            format!(
                r#"{{"number":{number},"html_url":"https://github.com/o/r/pull/{number}","merged_at":"{day}T00:00:00Z","updated_at":"2026-12-31T00:00:00Z","title":"Pull {number}","body":"Body","head":{{"sha":"sha{number}"}},"user":{{"login":"dev"}}}}"#
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

pub(super) fn pulls_body(numbers: &[i64]) -> String {
    let merged: Vec<(i64, &str)> = numbers
        .iter()
        .map(|number| (*number, "2026-01-05"))
        .collect();
    pulls_merged_on(&merged)
}

pub(super) fn estimate(status: ROIEstimateStatus) -> ROIEstimate {
    ROIEstimate {
        status,
        hours: match status {
            ROIEstimateStatus::Estimated => Some(3.5_f64),
            ROIEstimateStatus::NeedsReview | ROIEstimateStatus::Error => None,
        },
        reasoning: "Small change.".to_owned(),
        model: Some("gpt-5.2".to_owned()),
        evidence_source: Some("pr_metadata".to_owned()),
        effort_basis: Some("without_ai".to_owned()),
        cached: Some(false),
    }
}

pub(super) fn record(status: ROIEstimateStatus) -> ROIPullRecord {
    ROIPullRecord {
        repo: "o/r".to_owned(),
        number: 12,
        title: "Add the thing".to_owned(),
        url: "https://github.com/o/r/pull/12".to_owned(),
        login: "dev".to_owned(),
        emails: vec!["dev@example.com".to_owned()],
        profile_email: "dev@example.com".to_owned(),
        commit_emails: Some(vec!["work@example.com".to_owned()]),
        merged_at: "2026-01-05T00:00:00Z".to_owned(),
        head_sha: "abc123".to_owned(),
        additions: 10,
        deletions: 2,
        changed_files: 3,
        commit_count: 4,
        incomplete_metadata: true,
        estimate: estimate(status),
        cache_key: Some("key-1".to_owned()),
    }
}

pub(super) fn evidence() -> ROIPullEvidence {
    ROIPullEvidence {
        repo: "o/r".to_owned(),
        number: 12,
        title: "Add the thing".to_owned(),
        body: "Line one".to_owned(),
        url: "https://github.com/o/r/pull/12".to_owned(),
        login: "dev".to_owned(),
        emails: vec!["dev@example.com".to_owned()],
        profile_email: "dev@example.com".to_owned(),
        commit_emails: Some(vec!["work@example.com".to_owned()]),
        merged_at: "2026-01-05T00:00:00Z".to_owned(),
        head_sha: "abc123".to_owned(),
        additions: 10,
        deletions: 2,
        changed_files: 3,
        files: Vec::new(),
        commits: Vec::new(),
        commit_count: 4,
        incomplete_metadata: true,
    }
}

pub(super) fn processed(
    position: usize,
    status: ROIEstimateStatus,
    metadata_unavailable: bool,
) -> ProcessedPull {
    ProcessedPull {
        position,
        record: record(status),
        metadata_unavailable,
    }
}

pub(super) fn report() -> ROIReport {
    ROIReport {
        mode: "github".to_owned(),
        start: "2026-01-01".to_owned(),
        end: "2026-01-07".to_owned(),
        synced_at: "2026-01-07T00:00:00+00:00".to_owned(),
        repos: vec!["o/r".to_owned()],
        estimator_model: "gpt-5.2".to_owned(),
        estimator_prompt: "Estimate this.".to_owned(),
        effort_basis: "without_ai".to_owned(),
        spend: Vec::new(),
        pulls: vec![record(ROIEstimateStatus::Estimated)],
        settings_fingerprint: "fingerprint".to_owned(),
        warnings: None,
        unavailable_repos: None,
        id: None,
    }
}

#[derive(Debug, Error)]
pub(super) enum MemoryError {
    #[error("the memory repository refused the write")]
    Refused,
    #[error("the memory repository refused the read")]
    Unreadable,
}

#[derive(Default)]
pub(super) struct MemoryRepository {
    pub(super) params: Mutex<BTreeMap<String, JsonData>>,
    pub(super) writes: Mutex<Vec<(String, JsonData)>>,
    pub(super) refuse_writes: bool,
    pub(super) refuse_reads: bool,
}

impl MemoryRepository {
    pub(super) fn holding<Value: Serialize>(name: &str, value: &Value) -> Self {
        let data = serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap();
        let repository = Self::default();
        repository
            .params
            .lock()
            .unwrap()
            .insert(name.to_owned(), data);
        repository
    }

    pub(super) fn holding_text(name: &str, text: &str) -> Self {
        let repository = Self::default();
        repository
            .params
            .lock()
            .unwrap()
            .insert(name.to_owned(), serde_json::from_str(text).unwrap());
        repository
    }

    pub(super) fn written(&self) -> Vec<(String, JsonData)> {
        self.writes.lock().unwrap().clone()
    }
}

impl ReportRepository for MemoryRepository {
    type Error = MemoryError;

    fn get_param(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, MemoryError>> + Send {
        if self.refuse_reads {
            return ready(Err(MemoryError::Unreadable));
        }
        ready(Ok(self.params.lock().unwrap().get(param_name).cloned()))
    }

    fn set_param(
        &self,
        param_name: &str,
        param_value: JsonData,
    ) -> impl Future<Output = Result<(), MemoryError>> + Send {
        if self.refuse_writes {
            return ready(Err(MemoryError::Refused));
        }
        self.writes
            .lock()
            .unwrap()
            .push((param_name.to_owned(), param_value));
        ready(Ok(()))
    }
}

pub(super) struct NoCapabilities;

impl CapabilityPredicates for NoCapabilities {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
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

pub(super) fn failing_caller() -> CompletionCaller {
    Arc::new(|_request| {
        let future: CompletionFuture = Box::pin(ready(Err(SourceError::PaginationLimit)));
        future
    })
}

pub(super) fn answering_caller(content: &str) -> CompletionCaller {
    let quoted = serde_json::to_string(content).unwrap();
    let wire =
        format!(r#"{{"choices":[{{"finish_reason":"stop","message":{{"content":{quoted}}}}}]}}"#);
    Arc::new(move |_request| {
        let object: CompletionObject = RawValue::from_string(wire.clone()).unwrap();
        let future: CompletionFuture = Box::pin(ready(Ok(object)));
        future
    })
}
