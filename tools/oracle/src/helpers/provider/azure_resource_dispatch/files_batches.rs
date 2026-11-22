mod batches;
mod files;

use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) steps: Vec<Step>,
}

// One B694 or B541 call per step; every field the leaf needs arrives from the case
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    FileCreate(Fixture),
    FileContent(Fixture),
    FileRetrieve(Fixture),
    FileList(Fixture),
    FileDelete(Fixture),
    BatchCreate(Fixture),
    BatchRetrieve(Fixture),
    BatchCancel(Fixture),
    BatchList(Fixture),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Fixture {
    pub(super) api_base: String,
    pub(super) api_version: String,
    pub(super) api_key: String,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) target_id: String,
    #[serde(default)]
    pub(super) after: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<i64>,
}

impl Step {
    pub(super) const fn fixture(&self) -> &Fixture {
        match self {
            Self::FileCreate(value)
            | Self::FileContent(value)
            | Self::FileRetrieve(value)
            | Self::FileList(value)
            | Self::FileDelete(value)
            | Self::BatchCreate(value)
            | Self::BatchRetrieve(value)
            | Self::BatchCancel(value)
            | Self::BatchList(value) => value,
        }
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::FileCreate(_) => "file_create",
            Self::FileContent(_) => "file_content",
            Self::FileRetrieve(_) => "file_retrieve",
            Self::FileList(_) => "file_list",
            Self::FileDelete(_) => "file_delete",
            Self::BatchCreate(_) => "batch_create",
            Self::BatchRetrieve(_) => "batch_retrieve",
            Self::BatchCancel(_) => "batch_cancel",
            Self::BatchList(_) => "batch_list",
        }
    }

    const fn is_batch(&self) -> bool {
        matches!(
            self,
            Self::BatchCreate(_)
                | Self::BatchRetrieve(_)
                | Self::BatchCancel(_)
                | Self::BatchList(_)
        )
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let mut answers = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let value = if step.is_batch() {
            batches::answer(step)?
        } else {
            files::answer(step)?
        };
        answers.push(json!({"kind": step.kind(), "value": value}));
    }
    Ok(json!({"steps": answers}))
}
