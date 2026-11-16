use std::collections::BTreeMap;

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
                // B1534 leaves a mapping with a non-string value undecoded; LiteLLM appends an entry only for a JSON object
                if let Ok(member) = serde_json::from_str::<BTreeMap<String, TextOrOther>>(raw) {
                    data.push(ImageData {
                        b64_json: member.get("b64_json").and_then(text_of),
                        revised_prompt: None,
                        url: member.get("url").and_then(text_of),
                    });
                }
            }
        }
    }
    response
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TextOrOther {
    Text(String),
    Other(IgnoredAny),
}

fn text_of(value: &TextOrOther) -> Option<String> {
    match value {
        TextOrOther::Text(text) => Some(text.clone()),
        TextOrOther::Other(_) => None,
    }
}
