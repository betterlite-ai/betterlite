use std::collections::BTreeMap;

use betterlite_openapi::docs_settings::{DocsEnv, DocsUrls};
use betterlite_server::docs_routes::{DocsPage, DocsRouteConfig, page_body, redirect_location};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SwaggerPage {
        title: String,
        openapi_url: String,
        scenarios: Vec<RootPathScenario>,
    },
    DocsUrlResolution {
        scenarios: Vec<BTreeMap<String, String>>,
    },
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootPathScenario {
    server_root_path: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::SwaggerPage {
                title,
                openapi_url,
                scenarios,
            } => to_json(
                scenarios
                    .iter()
                    .map(|scenario| swagger_page(title, openapi_url, scenario))
                    .collect::<Vec<_>>(),
            ),
            Input::DocsUrlResolution { scenarios } => {
                to_json(scenarios.iter().map(resolution).collect::<Vec<_>>())
            }
        }
    }
}

fn swagger_page(title: &str, openapi_url: &str, scenario: &RootPathScenario) -> String {
    let config = DocsRouteConfig {
        urls: DocsUrls {
            docs_url: Some("/".to_owned()),
            redoc_url: None,
            openapi_url: Some(openapi_url.to_owned()),
            root_redirect_url: None,
        },
        title: title.to_owned(),
        server_root_path: scenario.server_root_path.clone(),
    };
    page_body(&config, openapi_url, DocsPage::Swagger)
}

fn resolution(scenario: &BTreeMap<String, String>) -> serde_json::Value {
    let env = DocsEnv {
        docs_url: scenario.get("DOCS_URL").cloned(),
        redoc_url: scenario.get("REDOC_URL").cloned(),
        openapi_url: scenario.get("OPENAPI_URL").cloned(),
        no_docs: scenario.get("NO_DOCS").cloned(),
        no_redoc: scenario.get("NO_REDOC").cloned(),
        no_openapi: scenario.get("NO_OPENAPI").cloned(),
        root_redirect_url: scenario.get("ROOT_REDIRECT_URL").cloned(),
    };
    let urls = env.resolve();
    let root_redirect_location = urls
        .root_redirect_url
        .as_deref()
        .map(|url| redirect_location(None, url));
    serde_json::json!({
        "docs_url": urls.docs_url,
        "redoc_url": urls.redoc_url,
        "openapi_url": urls.openapi_url,
        "root_redirect_url": urls.root_redirect_url,
        "root_redirect_location": root_redirect_location,
    })
}

fn to_json<T>(value: T) -> crate::helpers::HelperResult
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
