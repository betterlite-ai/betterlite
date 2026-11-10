use serde::{Deserialize, Serialize};

use crate::capabilities::CapabilityFields;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityFlag {
    Vision,
    FunctionCalling,
    ToolChoice,
    ResponseSchema,
    PromptCaching,
    Reasoning,
    WebSearch,
    PdfInput,
}

// Option<bool> keeps an absent flag distinct from an explicit false in every projection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityView {
    pub supports_vision: Option<bool>,
    pub supports_function_calling: Option<bool>,
    pub supports_tool_choice: Option<bool>,
    pub supports_response_schema: Option<bool>,
    pub supports_prompt_caching: Option<bool>,
    pub supports_reasoning: Option<bool>,
    pub supports_web_search: Option<bool>,
    pub supports_pdf_input: Option<bool>,
}

impl CapabilityView {
    #[must_use]
    pub const fn from_fields(fields: &CapabilityFields) -> Self {
        Self {
            supports_vision: fields.media.supports_vision,
            supports_function_calling: fields.tooling.supports_function_calling,
            supports_tool_choice: fields.tooling.supports_tool_choice,
            supports_response_schema: fields.tooling.supports_response_schema,
            supports_prompt_caching: fields.tooling.supports_prompt_caching,
            supports_reasoning: fields.reasoning.supports_reasoning,
            supports_web_search: fields.tooling.supports_web_search,
            supports_pdf_input: fields.media.supports_pdf_input,
        }
    }

    // Flags the entry leaves absent take the fill value, as the fill-missing backfill does.
    #[must_use]
    pub fn or(self, fill: Self) -> Self {
        Self {
            supports_vision: self.supports_vision.or(fill.supports_vision),
            supports_function_calling: self
                .supports_function_calling
                .or(fill.supports_function_calling),
            supports_tool_choice: self.supports_tool_choice.or(fill.supports_tool_choice),
            supports_response_schema: self
                .supports_response_schema
                .or(fill.supports_response_schema),
            supports_prompt_caching: self
                .supports_prompt_caching
                .or(fill.supports_prompt_caching),
            supports_reasoning: self.supports_reasoning.or(fill.supports_reasoning),
            supports_web_search: self.supports_web_search.or(fill.supports_web_search),
            supports_pdf_input: self.supports_pdf_input.or(fill.supports_pdf_input),
        }
    }

    #[must_use]
    pub const fn get(self, flag: CapabilityFlag) -> Option<bool> {
        match flag {
            CapabilityFlag::Vision => self.supports_vision,
            CapabilityFlag::FunctionCalling => self.supports_function_calling,
            CapabilityFlag::ToolChoice => self.supports_tool_choice,
            CapabilityFlag::ResponseSchema => self.supports_response_schema,
            CapabilityFlag::PromptCaching => self.supports_prompt_caching,
            CapabilityFlag::Reasoning => self.supports_reasoning,
            CapabilityFlag::WebSearch => self.supports_web_search,
            CapabilityFlag::PdfInput => self.supports_pdf_input,
        }
    }
}
