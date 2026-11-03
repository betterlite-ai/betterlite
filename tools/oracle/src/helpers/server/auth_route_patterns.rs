use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeError, ScopeFields, get_request_route, get_request_route_template,
};
use betterlite_auth::auth_route_patterns::normalize::RouteNormalizer;
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Scope { requests: Vec<RequestInput> },
    Normalize { routes: Vec<String> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    name: String,
    path: Option<String>,
    url_path: String,
    app_root_path: Option<String>,
    root_path: Option<String>,
    #[serde(default)]
    scope: Access,
    template: Option<String>,
    #[serde(default)]
    template_access: Access,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Access {
    #[default]
    Present,
    Malformed,
    Error,
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        if let Ok(mut levels) = self.0.lock() {
            levels.push(event.metadata().level().to_string());
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Scope { requests } => Ok(requests.iter().map(run_request).collect()),
            Input::Normalize { routes } => {
                let normalizer = RouteNormalizer::new().map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "NormalizeError".to_owned(),
                        message: error.to_string(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                    })
                })?;
                Ok(routes
                    .iter()
                    .map(|route| json!(normalizer.normalize_request_route(route)))
                    .collect())
            }
        }
    }
}

fn run_request(input: &RequestInput) -> serde_json::Value {
    let logs = Logs::default();
    let subscriber = Registry::default().with(logs.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let scope = match input.scope {
        Access::Present => Ok(Some(ScopeFields {
            path: input.path.as_deref(),
            app_root_path: input.app_root_path.as_deref(),
            root_path: input.root_path.as_deref(),
        })),
        Access::Malformed => Ok(None),
        Access::Error => Err(ScopeError::Access),
    };
    let route = get_request_route(&RequestRouteSource {
        scope,
        url_path: &input.url_path,
    });
    let template_access = match input.scope {
        Access::Present => &input.template_access,
        Access::Malformed | Access::Error => &input.scope,
    };
    let template = get_request_route_template(match template_access {
        Access::Present => Ok(input.template.as_deref()),
        Access::Malformed => Ok(None),
        Access::Error => Err(ScopeError::Access),
    });
    let levels = logs
        .0
        .lock()
        .map(|entries| entries.clone())
        .unwrap_or_default();
    json!({"name": input.name, "route": route, "template": template, "logs": levels})
}
