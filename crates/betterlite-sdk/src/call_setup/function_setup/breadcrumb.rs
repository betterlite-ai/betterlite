use betterlite_models::decisions::Json;

use super::CallSetupError;
use crate::call_setup::RequestMap;

pub trait BreadcrumbSink: Send + Sync {
    fn add_breadcrumb(&self, category: &str, message: &str, level: &str);
}

pub(super) fn add_call_breadcrumb(
    request: &RequestMap,
    sink: Option<&dyn BreadcrumbSink>,
    turn_off_message_logging: bool,
) -> Result<(), CallSetupError> {
    let Some(recorder) = sink else {
        return Ok(());
    };
    let mut details = request.clone();
    if turn_off_message_logging {
        for field in ["messages", "input", "prompt"] {
            details.shift_remove(field);
        }
    }
    let encoded = serde_json::to_string(&Json::Object(details))?;
    recorder.add_breadcrumb(
        "litellm.llm_call",
        &format!("Keyword Args: {encoded}"),
        "info",
    );
    Ok(())
}
