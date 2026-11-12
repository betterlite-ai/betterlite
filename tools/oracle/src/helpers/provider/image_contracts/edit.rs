use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_openai::api::image_contracts::records::{ImageFile, ImageFiles, ImageParams};
use betterlite_provider::api::image_contracts::edit::{BaseImageEditConfig, ImageEditRequest};
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::fixtures::{EditFixture, Seen};
use super::{AsyncCopyScenario, EditAsyncTransformCopy, MODEL};

pub(super) fn async_copy(fields: &EditAsyncTransformCopy) -> crate::helpers::HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| Box::<HelperError>::from(source))?;
    let cases: Vec<Value> = fields
        .scenarios
        .iter()
        .map(|scenario| copy_scenario(&runtime, scenario))
        .collect();
    Ok(json!({"cases": cases}))
}

fn copy_scenario(runtime: &tokio::runtime::Runtime, scenario: &AsyncCopyScenario) -> Value {
    let fixture = EditFixture {
        calls: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
        raise_status: scenario.raise_status,
    };
    let original_options = scenario.options.clone();
    let original_headers = scenario.headers.clone();
    let request = ImageEditRequest {
        model: &scenario.model,
        prompt: scenario.prompt.as_deref(),
        image: None,
        optional_params: scenario.options.clone(),
        litellm_params: &scenario.litellm_params,
        headers: scenario.headers.clone(),
    };
    let outcome = runtime.block_on(fixture.async_transform_image_edit_request(request));
    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let calls = fixture.calls.load(Ordering::Relaxed);
    let result = match outcome {
        Ok((data, files)) => json!({"data": data, "files": files_value(&files)}),
        Err(error) => json!({"raised": error.to_string()}),
    };
    json!({
        "calls": calls,
        "seen": seen_values(&seen),
        "options": original_options,
        "headers": original_headers,
        "result": result,
    })
}

fn seen_values(seen: &[Seen]) -> Vec<Value> {
    seen.iter()
        .map(|record| {
            json!({
                "model": record.model,
                "prompt": record.prompt,
                "image": record.image.as_ref().map(image_value),
                "options": record.options,
                "headers": record.headers,
            })
        })
        .collect()
}

fn image_value(image: &ImageFile) -> Value {
    json!({
        "filename": image.filename,
        "content": image.content,
        "content_type": image.content_type,
    })
}

fn files_value(files: &ImageFiles) -> Vec<Value> {
    files
        .iter()
        .map(|(name, file)| {
            json!({
                "name": name,
                "filename": file.filename,
                "content": file.content,
                "content_type": file.content_type,
            })
        })
        .collect()
}
