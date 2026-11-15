use betterlite_policy::pipeline::GuardrailPipeline;
use betterlite_policy::records::JsonObject;

pub(super) fn validate_pipeline(
    pipeline: Option<&JsonObject>,
) -> Result<Option<GuardrailPipeline>, serde_json::Error> {
    pipeline
        .map(|raw| {
            serde_json::to_string(raw)
                .and_then(|text| serde_json::from_str::<GuardrailPipeline>(&text))
        })
        .transpose()
}
