use betterlite_guardrails::prompt_injection::hook::PromptInjectionDetection;

use crate::startup_checks::error::StartupCheckError;

pub fn attach_router_to_detectors(
    detectors: &mut [PromptInjectionDetection],
    model_names: Option<&[String]>,
) -> Result<(), StartupCheckError> {
    for detector in detectors {
        detector.update_environment(model_names)?;
    }
    Ok(())
}
