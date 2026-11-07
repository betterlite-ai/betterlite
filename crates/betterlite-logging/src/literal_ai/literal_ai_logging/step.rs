use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

// The step dict _prepare_log_data builds, in the order the GraphQL variables read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: Json,
    pub error: Option<String>,
    pub name: String,
    pub thread_id: Option<Json>,
    pub parent_id: Option<Json>,
    pub root_run_id: Option<Json>,
    pub input: Option<Json>,
    pub output: Option<Json>,
    pub step_type: String,
    pub tags: Option<Json>,
    pub start_time: String,
    pub end_time: String,
    pub metadata: Json,
    pub generation: Generation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generation {
    pub input_token_count: i64,
    pub output_token_count: i64,
    pub token_count: i64,
    pub prompt_id: Option<Json>,
    pub variables: Option<Json>,
    pub provider: String,
    pub model: String,
    pub duration: Number,
    pub settings: Json,
    pub messages: Option<Json>,
    pub message_completion: Option<Json>,
    pub tools: Option<Json>,
}

impl Step {
    #[must_use]
    pub fn to_fields(&self) -> IndexMap<String, Json> {
        let mut fields = IndexMap::new();
        fields.insert("id".to_owned(), self.id.clone());
        fields.insert(
            "error".to_owned(),
            self.error.clone().map_or(Json::Null, Json::Text),
        );
        fields.insert("name".to_owned(), Json::Text(self.name.clone()));
        fields.insert(
            "threadId".to_owned(),
            self.thread_id.clone().unwrap_or(Json::Null),
        );
        fields.insert(
            "parentId".to_owned(),
            self.parent_id.clone().unwrap_or(Json::Null),
        );
        fields.insert(
            "rootRunId".to_owned(),
            self.root_run_id.clone().unwrap_or(Json::Null),
        );
        fields.insert("input".to_owned(), self.input.clone().unwrap_or(Json::Null));
        fields.insert(
            "output".to_owned(),
            self.output.clone().unwrap_or(Json::Null),
        );
        fields.insert("type".to_owned(), Json::Text(self.step_type.clone()));
        fields.insert("tags".to_owned(), self.tags.clone().unwrap_or(Json::Null));
        fields.insert("startTime".to_owned(), Json::Text(self.start_time.clone()));
        fields.insert("endTime".to_owned(), Json::Text(self.end_time.clone()));
        fields.insert("metadata".to_owned(), self.metadata.clone());
        fields.insert(
            "generation".to_owned(),
            Json::Object(self.generation.to_fields()),
        );
        fields
    }
}

impl Generation {
    #[must_use]
    pub fn to_fields(&self) -> IndexMap<String, Json> {
        let mut fields = IndexMap::new();
        fields.insert(
            "inputTokenCount".to_owned(),
            Json::Number(Number::from(self.input_token_count)),
        );
        fields.insert(
            "outputTokenCount".to_owned(),
            Json::Number(Number::from(self.output_token_count)),
        );
        fields.insert(
            "tokenCount".to_owned(),
            Json::Number(Number::from(self.token_count)),
        );
        fields.insert(
            "promptId".to_owned(),
            self.prompt_id.clone().unwrap_or(Json::Null),
        );
        fields.insert(
            "variables".to_owned(),
            self.variables.clone().unwrap_or(Json::Null),
        );
        fields.insert("provider".to_owned(), Json::Text(self.provider.clone()));
        fields.insert("model".to_owned(), Json::Text(self.model.clone()));
        fields.insert("duration".to_owned(), Json::Number(self.duration.clone()));
        fields.insert("settings".to_owned(), self.settings.clone());
        fields.insert(
            "messages".to_owned(),
            self.messages.clone().unwrap_or(Json::Null),
        );
        fields.insert(
            "messageCompletion".to_owned(),
            self.message_completion.clone().unwrap_or(Json::Null),
        );
        fields.insert("tools".to_owned(), self.tools.clone().unwrap_or(Json::Null));
        fields
    }
}
