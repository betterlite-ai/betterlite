use betterlite_config::sections::fallback_errors::FallbackErrors;
use betterlite_models::model_registration::JsonObject;

pub fn drop_fallback_errors_flag(section: &FallbackErrors, data: &mut JsonObject) {
    if !section.expose_to_caller {
        data.shift_remove("include_fallback_errors");
    }
}
