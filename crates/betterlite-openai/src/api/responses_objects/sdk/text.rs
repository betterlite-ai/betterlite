use crate::api::responses_shapes::FunctionCallStatus;
use crate::api::responses_shapes::ResponsePhase;

record! { OutputMessage: OutputMessageTag = "message" {
    id: String,
    content: Vec<super::unions::OutputMessageContentValue>,
    role: super::literals::AssistantKind,
    status: FunctionCallStatus,
    phase: Option<ResponsePhase>,
} }
record! { OutputText: OutputTextTag = "output_text" {
    annotations: Vec<super::unions::OutputTextAnnotationsValue>,
    text: String,
    logprobs: Option<Vec<Logprob>>,
} }
record! { FileCitation: FileCitationTag = "file_citation" {
    file_id: String,
    filename: String,
    index: i64,
} }
record! { URLCitation: URLCitationTag = "url_citation" {
    end_index: i64,
    start_index: i64,
    title: String,
    url: String,
} }
record! { ContainerFileCitation: ContainerFileCitationTag = "container_file_citation" {
    container_id: String,
    end_index: i64,
    file_id: String,
    filename: String,
    start_index: i64,
} }
record! { FilePath: FilePathTag = "file_path" {
    file_id: String,
    index: i64,
} }
record! { Logprob {
    token: String,
    bytes: Vec<i64>,
    logprob: super::Number,
    top_logprobs: Vec<LogprobTopLogprob>,
} }
record! { LogprobTopLogprob {
    token: String,
    bytes: Vec<i64>,
    logprob: super::Number,
} }
record! { OutputRefusal: OutputRefusalTag = "refusal" {
    refusal: String,
} }
record! { Reasoning: ReasoningTag = "reasoning" {
    id: String,
    summary: Vec<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Vec<Content>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encrypted_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<FunctionCallStatus>,
} }
record! { Summary: SummaryTag = "summary_text" {
    text: String,
} }
record! { Content: ContentTag = "reasoning_text" {
    text: String,
} }

impl super::unions::SdkOutput {
    #[must_use]
    pub fn output_text(&self) -> String {
        if let Self::OutputMessage(message) = self {
            message
                .content
                .iter()
                .filter_map(|part| match part {
                    super::unions::OutputMessageContentValue::OutputText(text) => {
                        Some(text.text.as_str())
                    }
                    super::unions::OutputMessageContentValue::OutputRefusal(_) => None,
                })
                .collect()
        } else {
            String::new()
        }
    }
}
