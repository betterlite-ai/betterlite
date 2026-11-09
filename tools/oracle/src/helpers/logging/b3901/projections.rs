use betterlite_logging::langfuse_observations::attributes::{
    ObservationAttributes, TraceAttributes, observation_attributes,
};

use super::inputs::{ObservationFields, TraceFields};
use super::values::pairs;

impl<'fields> From<&'fields TraceFields> for TraceAttributes<'fields> {
    fn from(fields: &'fields TraceFields) -> Self {
        Self {
            name: fields.name.as_ref(),
            user_id: fields.user_id.as_ref(),
            session_id: fields.session_id.as_ref(),
            version: fields.version.as_ref(),
            release: fields.release.as_ref(),
            tags: fields.tags.as_ref(),
            metadata: fields.metadata.as_ref(),
            public: fields.public,
            input: fields.input.as_ref(),
            output: fields.output.as_ref(),
        }
    }
}

pub(crate) fn observation_projection(fields: &ObservationFields) -> serde_json::Value {
    pairs(&observation_attributes(&ObservationAttributes {
        observation_type: fields.observation_type,
        input: fields.input.as_ref(),
        output: fields.output.as_ref(),
        metadata: fields.metadata.as_ref(),
        level: fields.level.as_ref(),
        status_message: fields.status_message.as_ref(),
        version: fields.version.as_ref(),
        model: fields.model.as_ref(),
        model_parameters: fields.model_parameters.as_ref(),
        usage_details: fields.usage_details.as_ref(),
        cost_details: fields.cost_details.as_ref(),
        completion_start_time: fields.completion_start_time,
        prompt: fields.prompt.as_ref(),
    }))
}
