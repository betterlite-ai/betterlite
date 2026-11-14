from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class FunctionResponse(TypedDict, total=False):
    id: str
    name: Required[str]
    response: dict[str, JsonValue] | None
    parts: list["FunctionResponsePartType"]


class FunctionCall(TypedDict, total=False):
    id: str
    name: Required[str]
    args: dict[str, JsonValue] | None


class FileDataType(TypedDict):
    mime_type: str
    file_uri: str


class BlobType(TypedDict, total=False):
    mime_type: Required[str]
    data: Required[str]


class FunctionResponsePartType(TypedDict, total=False):
    inline_data: BlobType
    file_data: FileDataType


class HttpxFunctionCall(TypedDict, total=False):
    id: str
    name: Required[str]
    args: dict[str, JsonValue]


class HttpxExecutableCode(TypedDict):
    code: str
    language: str


class HttpxCodeExecutionResult(TypedDict):
    outcome: str
    output: str


class HttpxBlobType(TypedDict, total=False):
    mimeType: str
    data: str


class HttpxServerSideToolCall(TypedDict, total=False):
    toolType: str
    id: str
    args: dict[str, JsonValue]


class HttpxServerSideToolResponse(TypedDict, total=False):
    toolType: str
    id: str
    response: str | dict[str, JsonValue]


class HttpxPartType(TypedDict, total=False):
    text: str
    inlineData: HttpxBlobType
    fileData: FileDataType
    functionCall: HttpxFunctionCall
    functionResponse: FunctionResponse
    toolCall: HttpxServerSideToolCall
    toolResponse: HttpxServerSideToolResponse
    executableCode: HttpxExecutableCode
    codeExecutionResult: HttpxCodeExecutionResult
    thought: bool
    thoughtSignature: str
    mediaResolution: Literal["low", "medium", "high"]


class HttpxContentType(TypedDict, total=False):
    role: Literal["user", "model"]
    parts: list[HttpxPartType]


class Schema(TypedDict, total=False):
    type: Literal["STRING", "INTEGER", "BOOLEAN", "NUMBER", "ARRAY", "OBJECT"]
    format: Literal["enum", "date-time"]
    title: str
    description: str
    nullable: bool
    default: JsonValue
    items: "Schema"
    minItems: str
    maxItems: str
    enum: list[str]
    properties: dict[str, "Schema"]
    propertyOrdering: list[str]
    required: list[str]
    minProperties: str
    maxProperties: str
    minimum: float
    maximum: float
    minLength: str
    maxLength: str
    pattern: str
    example: JsonValue
    anyOf: list["Schema"]


class FunctionDeclaration(TypedDict, total=False):
    name: Required[str]
    description: str
    parameters: Schema | dict[str, JsonValue]
    response: Schema


class VertexAISearch(TypedDict, total=False):
    datastore: Required[str]


class Retrieval(TypedDict):
    source: VertexAISearch


class GeminiThinkingConfig(TypedDict, total=False):
    includeThoughts: bool
    thinkingBudget: int
    thinkingLevel: Literal["minimal", "low", "medium", "high"]


GeminiResponseModalities = Literal["TEXT", "IMAGE", "AUDIO", "VIDEO"]

GeminiImageAspectRatio = Literal["1:1", "2:3", "3:2", "3:4", "4:3", "9:16", "16:9", "21:9"]

GeminiImageSize = Literal["1K", "2K", "4K"]


class GeminiImageConfig(TypedDict, total=False):
    aspectRatio: GeminiImageAspectRatio
    imageSize: GeminiImageSize


class PrebuiltVoiceConfig(TypedDict):
    voiceName: str


class VoiceConfig(TypedDict):
    prebuiltVoiceConfig: PrebuiltVoiceConfig


class SpeechConfig(TypedDict, total=False):
    voiceConfig: VoiceConfig
    languageCode: str


class GenerationConfig(TypedDict, total=False):
    temperature: float
    top_p: float
    top_k: float
    candidate_count: int
    max_output_tokens: int
    stop_sequences: list[str]
    presence_penalty: float
    frequency_penalty: float
    response_mime_type: Literal["text/plain", "application/json"]
    response_schema: dict[str, JsonValue]
    response_json_schema: dict[str, JsonValue]
    responseFormat: dict[str, JsonValue]
    seed: int
    responseLogprobs: bool
    logprobs: int
    responseModalities: list[GeminiResponseModalities]
    imageConfig: GeminiImageConfig
    thinkingConfig: GeminiThinkingConfig
    mediaResolution: str
    speechConfig: SpeechConfig


class Tools(TypedDict, total=False):
    function_declarations: list[FunctionDeclaration]
    googleSearch: dict[str, JsonValue]
    googleSearchRetrieval: dict[str, JsonValue]
    enterpriseWebSearch: dict[str, JsonValue]
    url_context: dict[str, JsonValue]
    code_execution: dict[str, JsonValue]
    googleMaps: dict[str, JsonValue]
    computerUse: dict[str, JsonValue]
    retrieval: Retrieval


class PromptTokensDetails(TypedDict):
    modality: Literal["TEXT", "AUDIO", "IMAGE", "VIDEO"]
    tokenCount: int


class UsageMetadata(TypedDict, total=False):
    promptTokenCount: int
    totalTokenCount: int
    candidatesTokenCount: int
    responseTokenCount: int
    cachedContentTokenCount: int
    toolUsePromptTokenCount: int
    toolUsePromptTokensDetails: list[PromptTokensDetails]
    promptTokensDetails: list[PromptTokensDetails]
    cacheTokensDetails: list[PromptTokensDetails]
    thoughtsTokenCount: int
    responseTokensDetails: list[PromptTokensDetails]
    candidatesTokensDetails: list[PromptTokensDetails]
