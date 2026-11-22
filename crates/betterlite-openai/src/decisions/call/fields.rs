use betterlite_models::decisions::Json;
use betterlite_models::decisions::openai::DecisionRequestBody as OpenAiRequestBody;
use betterlite_models::decisions::systemone::DecisionsRequestBody as SystemOneRequestBody;
use indexmap::IndexMap;
use serde::Serialize;

// LiteLLM's RequestFields: the endpoint picks the envelope it accepts before prepare_call sees it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestFields {
    SystemOne(SystemOneRequestFields),
    OpenAi(OpenAiDecisionRequestFields),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemOneRequestFields {
    pub state: Option<Json>,
    pub questions: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiDecisionRequestFields {
    pub input: Option<Json>,
    pub questions: Option<Vec<Json>>,
}

// The validated request keeps which envelope it came from, because the response is formatted back into it
#[derive(Debug)]
pub enum RequestFormat {
    SystemOne(SystemOneRequestBody),
    OpenAi(OpenAiRequestBody),
}

#[derive(Serialize)]
struct SystemOneEnvelope<'fields> {
    state: Option<&'fields Json>,
    questions: Option<&'fields IndexMap<String, Json>>,
}

#[derive(Serialize)]
struct OpenAiEnvelope<'fields> {
    input: Option<&'fields Json>,
    questions: Option<&'fields Vec<Json>>,
    safety_identifier: Option<&'fields str>,
}

impl RequestFields {
    // Both TypeAdapters LiteLLM builds validate a dict whose required values may be absent, so the envelope is serialized first
    pub fn validate(
        self,
        safety_identifier: Option<&str>,
    ) -> Result<RequestFormat, serde_json::Error> {
        match self {
            Self::SystemOne(fields) => {
                let envelope = SystemOneEnvelope {
                    state: fields.state.as_ref(),
                    questions: fields.questions.as_ref(),
                };
                let body: SystemOneRequestBody =
                    serde_json::from_slice(&serde_json::to_vec(&envelope)?)?;
                Ok(RequestFormat::SystemOne(body))
            }
            Self::OpenAi(fields) => {
                let envelope = OpenAiEnvelope {
                    input: fields.input.as_ref(),
                    questions: fields.questions.as_ref(),
                    safety_identifier,
                };
                let body: OpenAiRequestBody =
                    serde_json::from_slice(&serde_json::to_vec(&envelope)?)?;
                Ok(RequestFormat::OpenAi(body))
            }
        }
    }
}
