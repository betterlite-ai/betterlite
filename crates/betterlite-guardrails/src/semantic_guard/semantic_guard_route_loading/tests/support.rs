use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::runtime::Builder;

use crate::semantic_guard::semantic_guard_route_loading::builder::SemanticRouterBuilder;
use crate::semantic_guard::semantic_guard_route_loading::route::Route;
use crate::semantic_guard::semantic_guard_route_loading::sources::RouteSources;

pub(super) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(super) fn array(values: &[&str]) -> GuardrailValue {
    GuardrailValue::Array(values.iter().map(|value| text(value)).collect())
}

pub(super) fn object(fields: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

pub(super) fn route(
    name: &str,
    description: Option<&str>,
    utterances: &[&str],
    threshold: f64,
) -> Route {
    Route {
        name: name.to_owned(),
        description: description.unwrap_or_default().to_owned(),
        utterances: utterances.iter().map(|text| (*text).to_owned()).collect(),
        score_threshold: threshold,
    }
}

pub(super) fn environment(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(
        "DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD".to_owned(),
        value.to_owned(),
    )])
}

// the file is read before the future starts, so the test never blocks an async worker
pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

// the temp folder stays alive with the path it holds, so the loader can open the file
pub(super) fn routes_file(content: &str) -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("routes.yaml");
    fs::write(&path, content).unwrap();
    (folder, path)
}

pub(super) fn names(routes: &[Route]) -> Vec<&str> {
    routes.iter().map(|route| route.name.as_str()).collect()
}

pub(super) fn thresholds(routes: &[Route]) -> Vec<f64> {
    routes.iter().map(|route| route.score_threshold).collect()
}

pub(super) fn sources(
    templates: &[&str],
    file: Option<&Path>,
    inline: Vec<GuardrailValue>,
    threshold: f64,
) -> RouteSources {
    RouteSources {
        route_templates: templates.iter().map(|name| (*name).to_owned()).collect(),
        custom_routes_file: file.map(Path::to_path_buf),
        custom_routes: inline,
        global_threshold: threshold,
    }
}

pub(super) struct BuilderCall {
    pub(super) routes: Vec<Route>,
    pub(super) threshold: f64,
}

pub(super) struct FakeBuilder {
    pub(super) calls: Mutex<Vec<BuilderCall>>,
    fail: bool,
}

impl FakeBuilder {
    pub(super) fn passing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake semantic router build failed")]
pub(super) struct BuildFailure;

#[async_trait]
impl SemanticRouterBuilder for FakeBuilder {
    type Router = Vec<String>;

    type Error = BuildFailure;

    async fn build(
        &self,
        routes: Vec<Route>,
        global_threshold: f64,
    ) -> Result<Vec<String>, BuildFailure> {
        let names: Vec<String> = routes.iter().map(|route| route.name.clone()).collect();
        self.calls.lock().unwrap().push(BuilderCall {
            routes,
            threshold: global_threshold,
        });
        if self.fail {
            return Err(BuildFailure);
        }
        Ok(names)
    }
}
