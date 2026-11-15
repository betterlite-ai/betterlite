use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse};

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
                if let Ok(serde_json::Value::Object(object)) = serde_json::from_str(raw) {
                    data.push(ImageData {
                        b64_json: string_member(&object, "b64_json"),
                        revised_prompt: None,
                        url: string_member(&object, "url"),
                    });
                }
            }
        }
    }
    response
}

fn string_member(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}
