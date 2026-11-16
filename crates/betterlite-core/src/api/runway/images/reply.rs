use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse};
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::api::runway::tasks::record::{RunwayTask, TaskOutput, TaskOutputs};

// LiteLLM empties a missing data list before it reads the output, so a non-list or absent output still answers data: [].
#[must_use]
pub fn apply_task_output(task: &RunwayTask, mut response: ImageResponse) -> ImageResponse {
    let data = response.data.get_or_insert_with(Vec::new);
    let Some(TaskOutputs::Members(members)) = &task.output else {
        return response;
    };
    for member in members {
        match member {
            TaskOutput::Url(url) => data.push(ImageData {
                b64_json: None,
                revised_prompt: None,
                url: Some(url.clone()),
            }),
            TaskOutput::Fields(fields) => data.push(ImageData {
                b64_json: fields.get("b64_json").cloned(),
                revised_prompt: None,
                url: fields.get("url").cloned(),
            }),
            TaskOutput::Unsupported(raw) => {
                // B1534 leaves a mapping with a non-string value undecoded; LiteLLM still appends an entry for it
                if let Ok(member) = serde_json::from_str::<OutputMember>(raw) {
                    data.push(ImageData {
                        b64_json: text_of(member.b64_json),
                        revised_prompt: None,
                        url: text_of(member.url),
                    });
                }
            }
        }
    }
    response
}

#[derive(Debug, Deserialize)]
struct OutputMember {
    #[serde(default)]
    b64_json: Option<TextOrOther>,
    #[serde(default)]
    url: Option<TextOrOther>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TextOrOther {
    Text(String),
    Other(IgnoredAny),
}

fn text_of(value: Option<TextOrOther>) -> Option<String> {
    match value {
        Some(TextOrOther::Text(text)) => Some(text),
        Some(TextOrOther::Other(_)) | None => None,
    }
}
