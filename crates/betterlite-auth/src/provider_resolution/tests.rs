#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use rstest::rstest;
use serde_json::json;

use super::resolve_rate_limit_provider;
use crate::rejection_provider::{DeploymentSnapshot, ProviderResolution, SnapshotError};

struct NativeAnswer {
    resolved: &'static str,
    provider: &'static str,
}

struct FakeNative {
    answers: BTreeMap<&'static str, NativeAnswer>,
    calls: AtomicUsize,
}

impl FakeNative {
    fn new(entries: &[(&'static str, &'static str, &'static str)]) -> Self {
        let mut answers = BTreeMap::new();
        for (model, resolved, provider) in entries {
            answers.insert(
                *model,
                NativeAnswer {
                    resolved: *resolved,
                    provider: *provider,
                },
            );
        }
        Self {
            answers,
            calls: AtomicUsize::new(0),
        }
    }
}

impl ProviderResolution for FakeNative {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let Some(answer) = self.answers.get(model) else {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        };
        Ok(ResolvedModel {
            model: answer.resolved.to_owned(),
            provider: ProviderIdentity::Custom(answer.provider.to_owned()),
            api_base: None,
            api_key: None,
        })
    }
}

struct FakeRouter {
    deployments: Vec<Json>,
    reads: AtomicUsize,
}

impl FakeRouter {
    fn new(deployments: Vec<Json>) -> Self {
        Self {
            deployments,
            reads: AtomicUsize::new(0),
        }
    }
}

impl DeploymentSnapshot for FakeRouter {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(Some(self.deployments.as_slice()))
    }
}

fn deployment(alias: &str, underlying: &str) -> Json {
    serde_json::from_value(json!({
        "model_name": alias,
        "litellm_params": {"model": underlying},
    }))
    .expect("a deployment row is a JSON object")
}

fn rows(texts: &[&str]) -> Vec<Json> {
    texts
        .iter()
        .map(|text| serde_json::from_str(text).expect("a router row holds JSON"))
        .collect()
}

fn pair(model: &str, provider: &str) -> (String, String) {
    (model.to_owned(), provider.to_owned())
}

#[test]
fn native_resolution_skips_the_router() {
    let native = FakeNative::new(&[("openai/gpt-4o-mini", "gpt-4o-mini", "openai")]);
    let router = FakeRouter::new(vec![deployment("tpm-locked", "openai/gpt-4o-mini")]);
    let resolved = resolve_rate_limit_provider(Some("openai/gpt-4o-mini"), &native, Some(&router));
    assert_eq!(resolved, pair("gpt-4o-mini", "openai"));
    assert_eq!(native.calls.load(Ordering::Relaxed), 1);
    assert_eq!(router.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn router_alias_resolves_after_native_fails() {
    let native = FakeNative::new(&[("openai/gpt-4o-mini", "gpt-4o-mini", "openai")]);
    let router = FakeRouter::new(vec![deployment("tpm-locked", "openai/gpt-4o-mini")]);
    let resolved = resolve_rate_limit_provider(Some("tpm-locked"), &native, Some(&router));
    assert_eq!(resolved, pair("gpt-4o-mini", "openai"));
    assert_eq!(native.calls.load(Ordering::Relaxed), 2);
    assert_eq!(router.reads.load(Ordering::Relaxed), 1);
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
fn empty_model_returns_the_empty_pair(#[case] model: Option<&str>) {
    let native = FakeNative::new(&[]);
    let router = FakeRouter::new(vec![deployment("tpm-locked", "openai/gpt-4o-mini")]);
    let resolved = resolve_rate_limit_provider(model, &native, Some(&router));
    assert_eq!(resolved, pair("", "litellm_proxy"));
    assert_eq!(native.calls.load(Ordering::Relaxed), 0);
    assert_eq!(router.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn unresolved_model_preserved_with_empty_router() {
    let native = FakeNative::new(&[]);
    let router = FakeRouter::new(Vec::new());
    let resolved = resolve_rate_limit_provider(Some("tpm-locked"), &native, Some(&router));
    assert_eq!(resolved, pair("tpm-locked", "litellm_proxy"));
    assert_eq!(native.calls.load(Ordering::Relaxed), 1);
    assert_eq!(router.reads.load(Ordering::Relaxed), 1);
}

#[test]
fn absent_router_falls_back_after_native_failure() {
    let native = FakeNative::new(&[("openai/gpt-4o-mini", "gpt-4o-mini", "openai")]);
    let resolved = resolve_rate_limit_provider(Some("tpm-locked"), &native, None);
    assert_eq!(resolved, pair("tpm-locked", "litellm_proxy"));
    assert_eq!(native.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn malformed_alias_rows_fall_back() {
    let native = FakeNative::new(&[]);
    let router = FakeRouter::new(rows(&[
        r#""not-a-deployment""#,
        r#"{"model_name": 7, "litellm_params": {"model": "openai/gpt-4o-mini"}}"#,
        r#"{"model_name": "tpm-locked"}"#,
        r#"{"model_name": "tpm-locked", "litellm_params": "openai/gpt-4o-mini"}"#,
        r#"{"model_name": "tpm-locked", "litellm_params": {"model": 7}}"#,
        r#"{"model_name": "tpm-locked", "litellm_params": {"model": ""}}"#,
    ]));
    let resolved = resolve_rate_limit_provider(Some("tpm-locked"), &native, Some(&router));
    assert_eq!(resolved, pair("tpm-locked", "litellm_proxy"));
}
