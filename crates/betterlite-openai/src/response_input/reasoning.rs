use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

// LiteLLM slices the content to 100 entries and then appends "...", which only a list that short survives.
const REASONING_CONTENT_LIMIT: usize = 100;

pub(crate) fn handle_reasoning_item(item: &Object) -> Object {
    let fallback = LaxReasoningItem::from_source(item);
    let Some(candidate) = reasoning_candidate(item) else {
        return fallback.0;
    };
    let Ok(value) = serde_json::to_value(&candidate) else {
        return fallback.0;
    };
    match serde_json::from_value::<ReasoningItemRecord>(value) {
        Ok(record) => record.into_fields(),
        Err(error) => {
            tracing::debug!(error = %error, "a reasoning item failed validation; keeping its source fields");
            fallback.0
        }
    }
}

fn reasoning_candidate(item: &Object) -> Option<Object> {
    if item.contains_key("summary") {
        return Some(item.clone());
    }
    let Some(Json::List(summary)) = item.get("reasoning_content") else {
        return None;
    };
    if summary.len() > REASONING_CONTENT_LIMIT {
        return None;
    }
    let mut candidate = item.clone();
    candidate.insert("summary".to_owned(), Json::List(summary.clone()));
    Some(candidate)
}

// The source item LiteLLM keeps when validation fails; it drops these keys only when their value is null.
struct LaxReasoningItem(Object);

impl LaxReasoningItem {
    fn from_source(item: &Object) -> Self {
        let mut fields = Object::new();
        for (key, value) in item {
            if matches!(value, Json::Null)
                && matches!(key.as_str(), "status" | "content" | "encrypted_content")
            {
                continue;
            }
            fields.insert(key.clone(), value.clone());
        }
        Self(fields)
    }
}

#[derive(Debug, Deserialize)]
struct ReasoningItemRecord {
    id: String,
    summary: Vec<ReasoningSummary>,
    #[serde(rename = "type")]
    kind: ReasoningKind,
    #[serde(default)]
    content: Option<Vec<ReasoningContent>>,
    #[serde(default)]
    encrypted_content: Option<String>,
    #[serde(default)]
    status: Option<ReasoningStatus>,
    #[serde(flatten)]
    extra: Object,
}

impl ReasoningItemRecord {
    fn into_fields(self) -> Object {
        let mut fields = Object::new();
        fields.insert("id".to_owned(), Json::Text(self.id));
        fields.insert(
            "summary".to_owned(),
            Json::List(
                self.summary
                    .into_iter()
                    .map(|entry| Json::Object(entry.into_fields()))
                    .collect(),
            ),
        );
        fields.insert("type".to_owned(), Json::Text(self.kind.as_str().to_owned()));
        if let Some(content) = self.content {
            fields.insert(
                "content".to_owned(),
                Json::List(
                    content
                        .into_iter()
                        .map(|entry| Json::Object(entry.into_fields()))
                        .collect(),
                ),
            );
        }
        if let Some(encrypted_content) = self.encrypted_content {
            fields.insert(
                "encrypted_content".to_owned(),
                Json::Text(encrypted_content),
            );
        }
        if let Some(status) = self.status {
            fields.insert("status".to_owned(), Json::Text(status.as_str().to_owned()));
        }
        for (key, value) in self.extra {
            if !matches!(value, Json::Null) {
                fields.insert(key, value);
            }
        }
        fields
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningKind {
    Reasoning,
}

impl ReasoningKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Reasoning => "reasoning",
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReasoningSummary {
    text: String,
    #[serde(rename = "type")]
    kind: SummaryKind,
    #[serde(flatten)]
    extra: Object,
}

impl ReasoningSummary {
    fn into_fields(self) -> Object {
        let mut fields = Object::new();
        fields.insert("text".to_owned(), Json::Text(self.text));
        fields.insert("type".to_owned(), Json::Text(self.kind.as_str().to_owned()));
        for (key, value) in self.extra {
            if !matches!(value, Json::Null) {
                fields.insert(key, value);
            }
        }
        fields
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SummaryKind {
    SummaryText,
}

impl SummaryKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SummaryText => "summary_text",
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReasoningContent {
    text: String,
    #[serde(rename = "type")]
    kind: ContentKind,
    #[serde(flatten)]
    extra: Object,
}

impl ReasoningContent {
    fn into_fields(self) -> Object {
        let mut fields = Object::new();
        fields.insert("text".to_owned(), Json::Text(self.text));
        fields.insert("type".to_owned(), Json::Text(self.kind.as_str().to_owned()));
        for (key, value) in self.extra {
            if !matches!(value, Json::Null) {
                fields.insert(key, value);
            }
        }
        fields
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContentKind {
    ReasoningText,
}

impl ContentKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReasoningText => "reasoning_text",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningStatus {
    InProgress,
    Completed,
    Incomplete,
}

impl ReasoningStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Incomplete => "incomplete",
        }
    }
}
