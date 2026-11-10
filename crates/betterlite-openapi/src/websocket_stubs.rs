use serde::{Deserialize, Serialize};

use crate::schema_transforms::tree::{JsonObject, JsonTree};

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebsocketRoute {
    pub path: String,
    pub name: Option<String>,
    pub query_params: Vec<QueryParam>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryParam {
    pub name: String,
    pub required: bool,
}

pub fn inject_websocket_stubs(document: &mut JsonObject, routes: &[WebsocketRoute]) {
    for route in routes {
        let base_path = stub_path(&route.path);
        let Some(JsonTree::Object(paths)) = document.get_mut("paths") else {
            return;
        };
        let path_entry = paths
            .entry(base_path.to_owned())
            .or_insert_with(|| JsonTree::Object(JsonObject::new()));
        let JsonTree::Object(path_operations) = path_entry else {
            continue;
        };
        if path_operations.contains_key("get") {
            continue;
        }
        path_operations.insert("get".to_owned(), websocket_stub(route, base_path));
    }
}

fn stub_path(path: &str) -> &str {
    path.split_once('{')
        .map_or(path, |(before, _)| before)
        .trim_end_matches('?')
}

fn websocket_stub(route: &WebsocketRoute, base_path: &str) -> JsonTree {
    let summary_name = route.name.as_deref().unwrap_or(base_path);
    let operation_id = route.name.as_deref().map_or_else(
        || format!("websocket_{}", base_path.replace('/', "_")),
        |name| format!("websocket_{name}"),
    );
    JsonTree::Object(JsonObject::from([
        (
            "summary".to_owned(),
            JsonTree::Text(format!("WebSocket: {summary_name}")),
        ),
        (
            "description".to_owned(),
            JsonTree::Text("WebSocket connection endpoint".to_owned()),
        ),
        ("operationId".to_owned(), JsonTree::Text(operation_id)),
        (
            "parameters".to_owned(),
            query_parameters(&route.query_params),
        ),
        ("responses".to_owned(), websocket_responses()),
        (
            "tags".to_owned(),
            JsonTree::Array(vec![JsonTree::Text("WebSocket".to_owned())]),
        ),
    ]))
}

fn query_parameters(query_params: &[QueryParam]) -> JsonTree {
    let mut parameters = Vec::with_capacity(query_params.len());
    for param in query_params {
        parameters.push(JsonTree::Object(JsonObject::from([
            ("name".to_owned(), JsonTree::Text(param.name.clone())),
            ("in".to_owned(), JsonTree::Text("query".to_owned())),
            ("required".to_owned(), JsonTree::Flag(param.required)),
            (
                "schema".to_owned(),
                JsonTree::Object(JsonObject::from([(
                    "type".to_owned(),
                    JsonTree::Text("string".to_owned()),
                )])),
            ),
        ])));
    }
    JsonTree::Array(parameters)
}

fn websocket_responses() -> JsonTree {
    JsonTree::Object(JsonObject::from([(
        "101".to_owned(),
        JsonTree::Object(JsonObject::from([(
            "description".to_owned(),
            JsonTree::Text("WebSocket Protocol Switched".to_owned()),
        )])),
    )]))
}
