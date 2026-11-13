use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// _merge_tools_from_deployment: the deployment's tools first, then the request's. An empty pair leaves the key alone.
#[must_use]
pub fn merged_tools(deployment: &[Json], request: &[Json]) -> Option<Vec<Json>> {
    if deployment.is_empty() && request.is_empty() {
        return None;
    }
    let mut tools = Vec::with_capacity(deployment.len().saturating_add(request.len()));
    tools.extend_from_slice(deployment);
    tools.extend_from_slice(request);
    Some(tools)
}

// what the deployment update copies; a clientside request passes its per-request deployment here
pub struct DeploymentView<'a> {
    pub model_name: &'a str,
    pub model: &'a str,
    pub api_base: Option<&'a Json>,
    pub model_info: &'a JsonObject,
    pub tags: &'a [String],
    pub credential_name: Option<&'a str>,
    pub tools: &'a [Json],
    pub tool_choice: Option<&'a Json>,
}

// The parts of a call's kwargs this update owns: the metadata bucket, the top-level model_info, tags and tools.
#[derive(Clone, Debug, Default)]
pub struct RequestContext {
    pub metadata: JsonObject,
    pub model_info: JsonObject,
    pub tags: Vec<String>,
    pub tools: Option<Vec<Json>>,
    pub tool_choice: Option<Json>,
}

impl RequestContext {
    pub fn update(&mut self, deployment: &DeploymentView<'_>) {
        if let Some(tools) =
            merged_tools(deployment.tools, self.tools.as_deref().unwrap_or_default())
        {
            self.tools = Some(tools);
        }
        if self.tool_choice.is_none() {
            self.tool_choice = deployment.tool_choice.cloned();
        }
        self.metadata.insert(
            "deployment".to_owned(),
            Json::Text(deployment.model.to_owned()),
        );
        self.metadata.insert(
            "model_info".to_owned(),
            Json::Object(deployment.model_info.clone()),
        );
        self.metadata.insert(
            "api_base".to_owned(),
            deployment.api_base.cloned().unwrap_or(Json::Null),
        );
        self.metadata.insert(
            "deployment_model_name".to_owned(),
            Json::Text(deployment.model_name.to_owned()),
        );
        self.model_info = deployment.model_info.clone();
        for tag in deployment.tags {
            if !self.tags.iter().any(|existing| existing == tag) {
                self.tags.push(tag.clone());
            }
        }
        if let Some(name) = deployment.credential_name {
            let credential_tag = format!("Credential: {name}");
            if !self.tags.iter().any(|existing| *existing == credential_tag) {
                self.tags.push(credential_tag);
            }
        }
        if !self.tags.is_empty() {
            self.metadata.insert(
                "tags".to_owned(),
                Json::List(self.tags.iter().cloned().map(Json::Text).collect()),
            );
        }
    }
}
