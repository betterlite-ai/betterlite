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
        let fields = [
            ("id", self.id.clone()),
            ("error", self.error.clone().map_or(Json::Null, Json::Text)),
            ("name", Json::Text(self.name.clone())),
            ("threadId", self.thread_id.clone().unwrap_or(Json::Null)),
            ("parentId", self.parent_id.clone().unwrap_or(Json::Null)),
            ("rootRunId", self.root_run_id.clone().unwrap_or(Json::Null)),
            ("input", self.input.clone().unwrap_or(Json::Null)),
            ("output", self.output.clone().unwrap_or(Json::Null)),
            ("type", Json::Text(self.step_type.clone())),
            ("tags", self.tags.clone().unwrap_or(Json::Null)),
            ("startTime", Json::Text(self.start_time.clone())),
            ("endTime", Json::Text(self.end_time.clone())),
            ("metadata", self.metadata.clone()),
            ("generation", Json::Object(self.generation.to_fields())),
        ];
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
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
