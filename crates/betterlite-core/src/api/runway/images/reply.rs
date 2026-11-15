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
            TaskOutput::Unsupported(_) => {}
        }
    }
    response
}
