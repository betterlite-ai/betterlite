from enum import Enum
from typing import ClassVar, Literal, TypeAlias

from pydantic import BaseModel, ConfigDict, Field, JsonValue, SerializeAsAny

OCIRoles: TypeAlias = Literal["SYSTEM", "USER", "ASSISTANT", "TOOL"]


class _OCIModel(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", serialize_by_alias=True)


class OCIVendors(Enum):
    COHERE = "COHERE"
    GENERIC = "GENERIC"


class OCIContentPart(_OCIModel):
    pass


class OCITextContentPart(OCIContentPart):
    type: Literal["TEXT"] = "TEXT"
    text: str


class OCIImageUrl(_OCIModel):
    url: str
    detail: Literal["AUTO", "HIGH", "LOW"] | None = None


class OCIImageContentPart(OCIContentPart):
    type: Literal["IMAGE"] = "IMAGE"
    image_url: OCIImageUrl = Field(alias="imageUrl")


OCIContentPartUnion: TypeAlias = OCITextContentPart | OCIImageContentPart


class OCIToolCall(_OCIModel):
    id: str | None = None
    type: Literal["FUNCTION"] = "FUNCTION"
    name: str
    arguments: str


class OCIToolDefinition(_OCIModel):
    type: Literal["FUNCTION"] = "FUNCTION"
    name: str | None = None
    description: str | None = None
    parameters: dict[str, JsonValue] | None = None


class OCIMessage(_OCIModel):
    role: OCIRoles
    content: list[OCIContentPartUnion] | None = None
    tool_calls: list[OCIToolCall] | None = Field(default=None, alias="toolCalls")
    tool_call_id: str | None = Field(default=None, alias="toolCallId")


class OCIChatRequestPayload(_OCIModel):
    api_format: str = Field(alias="apiFormat")
    messages: list[OCIMessage]
    tools: list[OCIToolDefinition] | None = None
    is_stream: bool = Field(default=False, alias="isStream")
    num_generations: int | None = Field(default=None, alias="numGenerations")
    max_tokens: int | None = Field(default=None, alias="maxTokens")
    max_completion_tokens: int | None = Field(default=None, alias="maxCompletionTokens")
    temperature: float | None = None
    top_p: float | None = Field(default=None, alias="topP")
    stop: list[str] | None = None
    seed: int | None = None
    frequency_penalty: float | None = Field(default=None, alias="frequencyPenalty")
    presence_penalty: float | None = Field(default=None, alias="presencePenalty")
    reasoning_effort: str | None = Field(default=None, alias="reasoningEffort")
    response_format: dict[str, JsonValue] | None = Field(default=None, alias="responseFormat")
    tool_choice: str | dict[str, JsonValue] | None = Field(default=None, alias="toolChoice")
    logit_bias: dict[str, JsonValue] | None = Field(default=None, alias="logitBias")
    log_probs: int | None = Field(default=None, alias="logProbs")


class OCIServingMode(_OCIModel):
    serving_type: str = Field(alias="servingType")
    endpoint_id: str | None = Field(default=None, alias="endpointId")
    model_id: str | None = Field(default=None, alias="modelId")


class CohereToolCall(_OCIModel):
    name: str
    parameters: dict[str, JsonValue]


class CohereParameterDefinition(_OCIModel):
    description: str
    type: str
    is_required: bool = Field(default=False, alias="isRequired")


class CohereTool(_OCIModel):
    name: str
    description: str
    parameter_definitions: dict[str, CohereParameterDefinition] = Field(
        alias="parameterDefinitions")


class CohereToolResult(_OCIModel):
    call: CohereToolCall
    outputs: list[dict[str, JsonValue]]


class CohereMessage(_OCIModel):
    role: str
    message: str | None = None
    tool_calls: list[CohereToolCall] | None = Field(default=None, alias="toolCalls")


class CohereUserMessage(CohereMessage):
    role: str = Field(default="USER", pattern="^USER$", json_schema_extra={"const": "USER"})


class CohereChatBotMessage(CohereMessage):
    role: str = Field(
        default="CHATBOT", pattern="^CHATBOT$", json_schema_extra={"const": "CHATBOT"})


class CohereSystemMessage(CohereMessage):
    role: str = Field(default="SYSTEM", pattern="^SYSTEM$", json_schema_extra={"const": "SYSTEM"})


class CohereToolMessage(CohereMessage):
    role: str = Field(default="TOOL", pattern="^TOOL$", json_schema_extra={"const": "TOOL"})
    tool_results: list[CohereToolResult] = Field(alias="toolResults")


class CohereChatRequest(_OCIModel):
    message: str
    api_format: Literal["COHERE"] = Field(default="COHERE", alias="apiFormat")
    chat_history: list[SerializeAsAny[CohereMessage]] | None = Field(
        default=None, alias="chatHistory")
    max_tokens: int | None = Field(default=None, alias="maxTokens")
    temperature: float | None = None
    top_p: float | None = Field(default=None, alias="topP")
    top_k: int | None = Field(default=None, alias="topK")
    frequency_penalty: float | None = Field(default=None, alias="frequencyPenalty")
    presence_penalty: float | None = Field(default=None, alias="presencePenalty")
    stop_sequences: list[str] | None = Field(default=None, alias="stopSequences")
    seed: int | None = None
    tools: list[CohereTool] | None = None
    response_format: dict[str, JsonValue] | None = Field(default=None, alias="responseFormat")
    preamble_override: str | None = Field(default=None, alias="preambleOverride")
    documents: list[dict[str, JsonValue]] | None = None
    search_queries_only: bool | None = Field(default=None, alias="searchQueriesOnly")
    search_entry_point: str | None = Field(default=None, alias="searchEntryPoint")
    grounding: dict[str, JsonValue] | None = None
    is_echo: bool | None = Field(default=None, alias="isEcho")
    is_search_queries_only: bool | None = Field(default=None, alias="isSearchQueriesOnly")
    is_raw_prompting: bool | None = Field(default=None, alias="isRawPrompting")
    is_force_single_step: bool | None = Field(default=None, alias="isForceSingleStep")
    prompt_truncation: str | None = Field(default=None, alias="promptTruncation")
    safety_mode: str | None = Field(default=None, alias="safetyMode")
    citation_quality: str | None = Field(default=None, alias="citationQuality")
    max_input_tokens: int | None = Field(default=None, alias="maxInputTokens")
    is_stream: bool | None = Field(default=None, alias="isStream")
    stream_options: dict[str, JsonValue] | None = Field(default=None, alias="streamOptions")


class OCICompletionPayload(_OCIModel):
    compartment_id: str = Field(alias="compartmentId")
    serving_mode: OCIServingMode = Field(alias="servingMode")
    chat_request: OCIChatRequestPayload | CohereChatRequest = Field(alias="chatRequest")


class OCICompletionTokenDetails(_OCIModel):
    accepted_prediction_tokens: int | None = Field(default=None, alias="acceptedPredictionTokens")
    reasoning_tokens: int | None = Field(default=None, alias="reasoningTokens")


class OCIPromptTokensDetails(_OCIModel):
    cached_tokens: int | None = Field(default=None, alias="cachedTokens")


class OCIResponseUsage(_OCIModel):
    prompt_tokens: int = Field(alias="promptTokens")
    completion_tokens: int | None = Field(default=None, alias="completionTokens")
    total_tokens: int = Field(alias="totalTokens")
    completion_tokens_details: OCICompletionTokenDetails | None = Field(
        default=None, alias="completionTokensDetails")
    prompt_tokens_details: OCIPromptTokensDetails | None = Field(
        default=None, alias="promptTokensDetails")


class OCIResponseChoice(_OCIModel):
    index: int
    message: OCIMessage | None = None
    finish_reason: str | None = Field(default=None, alias="finishReason")
    logprobs: dict[str, JsonValue] | None = None


class OCIChatResponse(_OCIModel):
    api_format: str = Field(alias="apiFormat")
    time_created: str = Field(alias="timeCreated")
    choices: list[OCIResponseChoice]
    usage: OCIResponseUsage


class OCICompletionResponse(_OCIModel):
    model_id: str = Field(alias="modelId")
    model_version: str = Field(alias="modelVersion")
    chat_response: OCIChatResponse = Field(alias="chatResponse")


class OCIStreamDelta(_OCIModel):
    content: list[OCIContentPartUnion] | None = None
    role: str | None = None
    tool_calls: list[OCIToolCall] | None = Field(default=None, alias="toolCalls")


class OCIStreamChunk(_OCIModel):
    finish_reason: str | None = Field(default=None, alias="finishReason")
    message: OCIStreamDelta | None = None
    pad: str | None = None
    index: int | None = None


class CohereStreamChunk(_OCIModel):
    api_format: str = Field(alias="apiFormat")
    text: str | None = None
    chat_history: list[CohereMessage] | None = Field(default=None, alias="chatHistory")
    finish_reason: str | None = Field(default=None, alias="finishReason")
    tool_calls: list[CohereToolCall] | None = Field(default=None, alias="toolCalls")
    pad: str | None = None
    index: int | None = None


class CohereUsage(_OCIModel):
    prompt_tokens: int = Field(alias="promptTokens")
    completion_tokens: int = Field(alias="completionTokens")
    total_tokens: int = Field(alias="totalTokens")
    prompt_tokens_details: dict[str, JsonValue] | None = Field(
        default=None, alias="promptTokensDetails")
    completion_tokens_details: dict[str, JsonValue] | None = Field(
        default=None, alias="completionTokensDetails")


class CohereCitation(_OCIModel):
    start: int
    end: int
    text: str
    document_ids: list[str]


class CohereSearchQuery(_OCIModel):
    text: str
    generation_id: str


class CohereChatResponse(_OCIModel):
    text: str
    api_format: Literal["COHERE"] = Field(default="COHERE", alias="apiFormat")
    finish_reason: str | None = Field(default=None, alias="finishReason")
    chat_history: list[CohereMessage] | None = Field(default=None, alias="chatHistory")
    citations: list[CohereCitation] | None = None
    documents: list[dict[str, JsonValue]] | None = None
    error_message: str | None = Field(default=None, alias="errorMessage")
    is_search_required: bool | None = Field(default=None, alias="isSearchRequired")
    prompt: str | None = None
    search_queries: list[CohereSearchQuery] | None = Field(default=None, alias="searchQueries")
    tool_calls: list[CohereToolCall] | None = Field(default=None, alias="toolCalls")
    usage: CohereUsage | None = None


class CohereChatDetails(_OCIModel):
    compartment_id: str = Field(alias="compartmentId")
    serving_mode: OCIServingMode = Field(alias="servingMode")
    chat_request: CohereChatRequest = Field(alias="chatRequest")


class CohereChatResult(_OCIModel):
    model_id: str = Field(alias="modelId")
    model_version: str = Field(alias="modelVersion")
    chat_response: CohereChatResponse = Field(alias="chatResponse")


class OCIEmbedRequest(_OCIModel):
    compartment_id: str = Field(alias="compartmentId")
    serving_mode: OCIServingMode = Field(alias="servingMode")
    inputs: list[str]
    input_type: str | None = Field(default=None, alias="inputType")
    truncate: str | None = "END"
    output_dimensions: int | None = Field(default=None, alias="outputDimensions")


class OCIEmbedUsage(_OCIModel):
    prompt_tokens: int = Field(alias="promptTokens")
    total_tokens: int = Field(alias="totalTokens")


class OCIEmbedResponse(_OCIModel):
    id: str | None = None
    embeddings: list[list[float]]
    model_id: str = Field(alias="modelId")
    model_version: str = Field(alias="modelVersion")
    input_text_token_counts: list[int] | None = Field(default=None, alias="inputTextTokenCounts")
    usage: OCIEmbedUsage | None = None
