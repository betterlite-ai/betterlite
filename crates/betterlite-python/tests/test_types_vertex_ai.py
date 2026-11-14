import asyncio
import importlib
import inspect
from enum import Enum
from typing import ClassVar, Final, Literal, Protocol, TypeVar, get_args, get_origin

import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from typing_extensions import TypeForm, get_type_hints, is_typeddict

from betterlite.types.llms.vertex_ai.credentials import (
    VERTEX_AI_PROVIDER_METADATA_FIELDS,
    VERTEX_CREDENTIALS_TYPES,
    VertexAccessTokenResolver,
)
from betterlite.types.llms.vertex_ai_gemma import VertexGemmaContainerError

PACKAGE: Final = "betterlite.types.llms.vertex_ai"
REFERENCE_PACKAGE: Final = "litellm.types.llms.vertex_ai"


class TypedDictClass(Protocol):
    __required_keys__: ClassVar[frozenset[str]]
    __optional_keys__: ClassVar[frozenset[str]]
    __total__: ClassVar[bool]


class Blocks(Protocol):
    def __getattr__(self, name: str) -> TypedDictClass: ...


class Aliases(Protocol):
    def __getattr__(self, name: str) -> TypeForm[JsonValue]: ...


class StringTuples(Protocol):
    def __getattr__(self, name: str) -> tuple[str, ...]: ...


T_co = TypeVar("T_co", covariant=True)


class ClassesOf(Protocol[T_co]):
    def __getattr__(self, name: str) -> type[T_co]: ...


class ModelClasses(Protocol):
    def __getattr__(self, name: str) -> type[BaseModel]: ...


THEIRS: Blocks = importlib.import_module(REFERENCE_PACKAGE)
THEIR_ALIASES: Aliases = importlib.import_module(REFERENCE_PACKAGE)
THEIR_TUPLES: StringTuples = importlib.import_module(REFERENCE_PACKAGE)
THEIR_ENUMS: ClassesOf[Enum] = importlib.import_module(REFERENCE_PACKAGE)
THEIR_RESOLVERS: ClassesOf[VertexAccessTokenResolver] = importlib.import_module(
    REFERENCE_PACKAGE,
)
THEIR_GEMMA: ModelClasses = importlib.import_module("litellm.types.llms.vertex_ai_gemma")

CONTENT: Blocks = importlib.import_module(f"{PACKAGE}.content")
SCHEMA: Blocks = importlib.import_module(f"{PACKAGE}.schema")
TOOLS: Blocks = importlib.import_module(f"{PACKAGE}.tools")
SAFETY: Blocks = importlib.import_module(f"{PACKAGE}.safety")
GENERATION: Blocks = importlib.import_module(f"{PACKAGE}.generation")
CACHED: Blocks = importlib.import_module(f"{PACKAGE}.cached_content")
USAGE: Blocks = importlib.import_module(f"{PACKAGE}.usage")
RESPONSE: Blocks = importlib.import_module(f"{PACKAGE}.response")
TUNING: Blocks = importlib.import_module(f"{PACKAGE}.tuning")
EMBEDDINGS: Blocks = importlib.import_module(f"{PACKAGE}.embeddings")
MULTIMODAL: Blocks = importlib.import_module(f"{PACKAGE}.multimodal")
BATCH: Blocks = importlib.import_module(f"{PACKAGE}.batch")
VIDEO: Blocks = importlib.import_module(f"{PACKAGE}.video")

SAFETY_ALIASES: Aliases = importlib.import_module(f"{PACKAGE}.safety")
GENERATION_ALIASES: Aliases = importlib.import_module(f"{PACKAGE}.generation")
RESPONSE_ALIASES: Aliases = importlib.import_module(f"{PACKAGE}.response")
EMBEDDINGS_ALIASES: Aliases = importlib.import_module(f"{PACKAGE}.embeddings")
CREDENTIALS_ALIASES: Aliases = importlib.import_module(f"{PACKAGE}.credentials")

TOOLS_ENUMS: ClassesOf[Enum] = importlib.import_module(f"{PACKAGE}.tools")
EMBEDDINGS_ENUMS: ClassesOf[Enum] = importlib.import_module(f"{PACKAGE}.embeddings")
CREDENTIALS_ENUMS: ClassesOf[Enum] = importlib.import_module(f"{PACKAGE}.credentials")

LITELLM_NAMES: Final = frozenset(
    {
        "BlobType", "CachedContent", "CachedContentListAllResponseBody",
        "CachedContentRequestBody", "Candidates", "Citation", "CitationMetadata",
        "ContentEmbeddings", "ContentType", "Date", "EmbedContentRequest", "FileDataType",
        "FineTuneHyperparameters", "FineTuneJobCreate", "FineTunesupervisedTuningSpec",
        "FunctionCall", "FunctionCallingConfig", "FunctionDeclaration", "FunctionResponse",
        "FunctionResponsePartType", "GcsBucketResponse", "GcsDestination", "GcsSource",
        "GeminiEmbedContentRequestBody", "GeminiEmbedContentResponseObject",
        "GeminiEmbeddingInput", "GeminiFinishReason", "GeminiImageAspectRatio",
        "GeminiImageConfig", "GeminiImageSize", "GeminiResponseModalities",
        "GeminiThinkingConfig", "GenerateContentResponseBody", "GenerationConfig",
        "GroundingMetadata", "HarmBlockMethod", "HarmBlockThreshold", "HarmCategory",
        "HarmProbability", "HarmSeverity", "HttpxBlobType",
        "HttpxCodeExecutionResult", "HttpxContentType", "HttpxExecutableCode",
        "HttpxFunctionCall", "HttpxPartType", "HttpxServerSideToolCall",
        "HttpxServerSideToolResponse", "InputConfig", "Instance", "InstanceImage",
        "InstanceVideo", "LogprobsCandidate", "LogprobsResult", "LogprobsTopCandidate",
        "MultimodalPrediction", "MultimodalPredictions", "OutputConfig", "OutputInfo",
        "PartType", "PrebuiltVoiceConfig", "PromptFeedback", "PromptTokensDetails",
        "RequestBody", "ResponseSupervisedTuningSpec", "ResponseTuningJob", "Retrieval",
        "SafetSettingsConfig", "SafetyRatings", "Schema", "SearchEntryPoint",
        "SpeechConfig", "SystemInstructions", "TTL", "TaskTypeEnum",
        "TokenCountDetailsResponse", "ToolConfig", "Tools", "UrlContextMetadata",
        "UrlMetadata", "UsageMetadata", "VERTEX_AI_PROVIDER_METADATA_FIELDS",
        "VERTEX_CREDENTIALS_TYPES", "VertexAIBatchEmbeddingsRequestBody",
        "VertexAIBatchEmbeddingsResponseObject", "VertexAIBatchPredictionJob",
        "VertexAICachedContentResponseObject", "VertexAISearch",
        "VertexAITextEmbeddingsRequestBody", "VertexAITextEmbeddingsResponseObject",
        "VertexAccessTokenResolver", "VertexBatchPredictionResponse",
        "VertexMultimodalEmbeddingRequest", "VertexPartnerProvider", "VertexToolName",
        "VertexVideoGenerationInstance", "VertexVideoGenerationParameters",
        "VertexVideoGenerationRequest", "VertexVideoGenerationResponse",
        "VertexVideoImage", "VertexVideoOutput", "VideoEmbedding", "VideoSegmentConfig",
        "VoiceConfig",
    },
)

TYPED_DICT_PAIRS: Final[list[tuple[str, TypedDictClass, TypedDictClass]]] = [
    ("FileDataType", CONTENT.FileDataType, THEIRS.FileDataType),
    ("BlobType", CONTENT.BlobType, THEIRS.BlobType),
    ("FunctionResponsePartType", CONTENT.FunctionResponsePartType, THEIRS.FunctionResponsePartType),
    ("FunctionResponse", CONTENT.FunctionResponse, THEIRS.FunctionResponse),
    ("FunctionCall", CONTENT.FunctionCall, THEIRS.FunctionCall),
    ("PartType", CONTENT.PartType, THEIRS.PartType),
    ("HttpxFunctionCall", CONTENT.HttpxFunctionCall, THEIRS.HttpxFunctionCall),
    ("HttpxExecutableCode", CONTENT.HttpxExecutableCode, THEIRS.HttpxExecutableCode),
    ("HttpxCodeExecutionResult", CONTENT.HttpxCodeExecutionResult, THEIRS.HttpxCodeExecutionResult),
    ("HttpxBlobType", CONTENT.HttpxBlobType, THEIRS.HttpxBlobType),
    ("HttpxServerSideToolCall", CONTENT.HttpxServerSideToolCall, THEIRS.HttpxServerSideToolCall),
    (
        "HttpxServerSideToolResponse",
        CONTENT.HttpxServerSideToolResponse,
        THEIRS.HttpxServerSideToolResponse,
    ),
    ("HttpxPartType", CONTENT.HttpxPartType, THEIRS.HttpxPartType),
    ("HttpxContentType", CONTENT.HttpxContentType, THEIRS.HttpxContentType),
    ("ContentType", CONTENT.ContentType, THEIRS.ContentType),
    ("SystemInstructions", CONTENT.SystemInstructions, THEIRS.SystemInstructions),
    ("Schema", SCHEMA.Schema, THEIRS.Schema),
    ("FunctionDeclaration", SCHEMA.FunctionDeclaration, THEIRS.FunctionDeclaration),
    ("VertexAISearch", TOOLS.VertexAISearch, THEIRS.VertexAISearch),
    ("Retrieval", TOOLS.Retrieval, THEIRS.Retrieval),
    ("FunctionCallingConfig", TOOLS.FunctionCallingConfig, THEIRS.FunctionCallingConfig),
    ("Tools", TOOLS.Tools, THEIRS.Tools),
    ("ToolConfig", TOOLS.ToolConfig, THEIRS.ToolConfig),
    ("SafetSettingsConfig", SAFETY.SafetSettingsConfig, THEIRS.SafetSettingsConfig),
    ("GeminiThinkingConfig", GENERATION.GeminiThinkingConfig, THEIRS.GeminiThinkingConfig),
    ("GeminiImageConfig", GENERATION.GeminiImageConfig, THEIRS.GeminiImageConfig),
    ("PrebuiltVoiceConfig", GENERATION.PrebuiltVoiceConfig, THEIRS.PrebuiltVoiceConfig),
    ("VoiceConfig", GENERATION.VoiceConfig, THEIRS.VoiceConfig),
    ("SpeechConfig", GENERATION.SpeechConfig, THEIRS.SpeechConfig),
    ("GenerationConfig", GENERATION.GenerationConfig, THEIRS.GenerationConfig),
    ("TTL", CACHED.TTL, THEIRS.TTL),
    ("CachedContent", CACHED.CachedContent, THEIRS.CachedContent),
    ("RequestBody", CACHED.RequestBody, THEIRS.RequestBody),
    ("CachedContentRequestBody", CACHED.CachedContentRequestBody, THEIRS.CachedContentRequestBody),
    (
        "CachedContentListAllResponseBody",
        CACHED.CachedContentListAllResponseBody,
        THEIRS.CachedContentListAllResponseBody,
    ),
    (
        "VertexAICachedContentResponseObject",
        CACHED.VertexAICachedContentResponseObject,
        THEIRS.VertexAICachedContentResponseObject,
    ),
    ("PromptTokensDetails", USAGE.PromptTokensDetails, THEIRS.PromptTokensDetails),
    ("UsageMetadata", USAGE.UsageMetadata, THEIRS.UsageMetadata),
    (
        "TokenCountDetailsResponse",
        USAGE.TokenCountDetailsResponse,
        THEIRS.TokenCountDetailsResponse,
    ),
    ("SafetyRatings", RESPONSE.SafetyRatings, THEIRS.SafetyRatings),
    ("Date", RESPONSE.Date, THEIRS.Date),
    ("Citation", RESPONSE.Citation, THEIRS.Citation),
    ("CitationMetadata", RESPONSE.CitationMetadata, THEIRS.CitationMetadata),
    ("SearchEntryPoint", RESPONSE.SearchEntryPoint, THEIRS.SearchEntryPoint),
    ("GroundingMetadata", RESPONSE.GroundingMetadata, THEIRS.GroundingMetadata),
    ("LogprobsCandidate", RESPONSE.LogprobsCandidate, THEIRS.LogprobsCandidate),
    ("LogprobsTopCandidate", RESPONSE.LogprobsTopCandidate, THEIRS.LogprobsTopCandidate),
    ("LogprobsResult", RESPONSE.LogprobsResult, THEIRS.LogprobsResult),
    ("UrlMetadata", RESPONSE.UrlMetadata, THEIRS.UrlMetadata),
    ("UrlContextMetadata", RESPONSE.UrlContextMetadata, THEIRS.UrlContextMetadata),
    ("Candidates", RESPONSE.Candidates, THEIRS.Candidates),
    ("PromptFeedback", RESPONSE.PromptFeedback, THEIRS.PromptFeedback),
    (
        "GenerateContentResponseBody",
        RESPONSE.GenerateContentResponseBody,
        THEIRS.GenerateContentResponseBody,
    ),
    ("FineTuneHyperparameters", TUNING.FineTuneHyperparameters, THEIRS.FineTuneHyperparameters),
    (
        "FineTunesupervisedTuningSpec",
        TUNING.FineTunesupervisedTuningSpec,
        THEIRS.FineTunesupervisedTuningSpec,
    ),
    ("FineTuneJobCreate", TUNING.FineTuneJobCreate, THEIRS.FineTuneJobCreate),
    (
        "ResponseSupervisedTuningSpec",
        TUNING.ResponseSupervisedTuningSpec,
        THEIRS.ResponseSupervisedTuningSpec,
    ),
    ("ResponseTuningJob", TUNING.ResponseTuningJob, THEIRS.ResponseTuningJob),
    (
        "VertexAITextEmbeddingsRequestBody",
        EMBEDDINGS.VertexAITextEmbeddingsRequestBody,
        THEIRS.VertexAITextEmbeddingsRequestBody,
    ),
    ("ContentEmbeddings", EMBEDDINGS.ContentEmbeddings, THEIRS.ContentEmbeddings),
    (
        "VertexAITextEmbeddingsResponseObject",
        EMBEDDINGS.VertexAITextEmbeddingsResponseObject,
        THEIRS.VertexAITextEmbeddingsResponseObject,
    ),
    ("EmbedContentRequest", EMBEDDINGS.EmbedContentRequest, THEIRS.EmbedContentRequest),
    (
        "VertexAIBatchEmbeddingsRequestBody",
        EMBEDDINGS.VertexAIBatchEmbeddingsRequestBody,
        THEIRS.VertexAIBatchEmbeddingsRequestBody,
    ),
    (
        "VertexAIBatchEmbeddingsResponseObject",
        EMBEDDINGS.VertexAIBatchEmbeddingsResponseObject,
        THEIRS.VertexAIBatchEmbeddingsResponseObject,
    ),
    (
        "GeminiEmbedContentRequestBody",
        EMBEDDINGS.GeminiEmbedContentRequestBody,
        THEIRS.GeminiEmbedContentRequestBody,
    ),
    (
        "GeminiEmbedContentResponseObject",
        EMBEDDINGS.GeminiEmbedContentResponseObject,
        THEIRS.GeminiEmbedContentResponseObject,
    ),
    ("VideoSegmentConfig", MULTIMODAL.VideoSegmentConfig, THEIRS.VideoSegmentConfig),
    ("InstanceVideo", MULTIMODAL.InstanceVideo, THEIRS.InstanceVideo),
    ("InstanceImage", MULTIMODAL.InstanceImage, THEIRS.InstanceImage),
    ("Instance", MULTIMODAL.Instance, THEIRS.Instance),
    (
        "VertexMultimodalEmbeddingRequest",
        MULTIMODAL.VertexMultimodalEmbeddingRequest,
        THEIRS.VertexMultimodalEmbeddingRequest,
    ),
    ("VideoEmbedding", MULTIMODAL.VideoEmbedding, THEIRS.VideoEmbedding),
    ("MultimodalPrediction", MULTIMODAL.MultimodalPrediction, THEIRS.MultimodalPrediction),
    ("MultimodalPredictions", MULTIMODAL.MultimodalPredictions, THEIRS.MultimodalPredictions),
    ("GcsSource", BATCH.GcsSource, THEIRS.GcsSource),
    ("InputConfig", BATCH.InputConfig, THEIRS.InputConfig),
    ("GcsDestination", BATCH.GcsDestination, THEIRS.GcsDestination),
    ("OutputConfig", BATCH.OutputConfig, THEIRS.OutputConfig),
    ("OutputInfo", BATCH.OutputInfo, THEIRS.OutputInfo),
    ("GcsBucketResponse", BATCH.GcsBucketResponse, THEIRS.GcsBucketResponse),
    (
        "VertexAIBatchPredictionJob",
        BATCH.VertexAIBatchPredictionJob,
        THEIRS.VertexAIBatchPredictionJob,
    ),
    (
        "VertexBatchPredictionResponse",
        BATCH.VertexBatchPredictionResponse,
        THEIRS.VertexBatchPredictionResponse,
    ),
    ("VertexVideoImage", VIDEO.VertexVideoImage, THEIRS.VertexVideoImage),
    (
        "VertexVideoGenerationInstance",
        VIDEO.VertexVideoGenerationInstance,
        THEIRS.VertexVideoGenerationInstance,
    ),
    (
        "VertexVideoGenerationParameters",
        VIDEO.VertexVideoGenerationParameters,
        THEIRS.VertexVideoGenerationParameters,
    ),
    (
        "VertexVideoGenerationRequest",
        VIDEO.VertexVideoGenerationRequest,
        THEIRS.VertexVideoGenerationRequest,
    ),
    ("VertexVideoOutput", VIDEO.VertexVideoOutput, THEIRS.VertexVideoOutput),
    (
        "VertexVideoGenerationResponse",
        VIDEO.VertexVideoGenerationResponse,
        THEIRS.VertexVideoGenerationResponse,
    ),
]

ALIAS_PAIRS: Final[list[tuple[str, TypeForm[JsonValue], TypeForm[JsonValue]]]] = [
    (
        "HarmCategory",
        SAFETY_ALIASES.HarmCategory,
        THEIR_ALIASES.HarmCategory,
    ),
    (
        "HarmBlockThreshold",
        SAFETY_ALIASES.HarmBlockThreshold,
        THEIR_ALIASES.HarmBlockThreshold,
    ),
    (
        "HarmBlockMethod",
        SAFETY_ALIASES.HarmBlockMethod,
        THEIR_ALIASES.HarmBlockMethod,
    ),
    (
        "HarmProbability",
        SAFETY_ALIASES.HarmProbability,
        THEIR_ALIASES.HarmProbability,
    ),
    (
        "HarmSeverity",
        SAFETY_ALIASES.HarmSeverity,
        THEIR_ALIASES.HarmSeverity,
    ),
    (
        "GeminiResponseModalities",
        GENERATION_ALIASES.GeminiResponseModalities,
        THEIR_ALIASES.GeminiResponseModalities,
    ),
    (
        "GeminiImageAspectRatio",
        GENERATION_ALIASES.GeminiImageAspectRatio,
        THEIR_ALIASES.GeminiImageAspectRatio,
    ),
    (
        "GeminiImageSize",
        GENERATION_ALIASES.GeminiImageSize,
        THEIR_ALIASES.GeminiImageSize,
    ),
    (
        "GeminiFinishReason",
        RESPONSE_ALIASES.GeminiFinishReason,
        THEIR_ALIASES.GeminiFinishReason,
    ),
    (
        "GeminiEmbeddingInput",
        EMBEDDINGS_ALIASES.GeminiEmbeddingInput,
        THEIR_ALIASES.GeminiEmbeddingInput,
    ),
    (
        "VERTEX_CREDENTIALS_TYPES",
        CREDENTIALS_ALIASES.VERTEX_CREDENTIALS_TYPES,
        THEIR_ALIASES.VERTEX_CREDENTIALS_TYPES,
    ),
]

ENUM_PAIRS: Final[list[tuple[str, type[Enum], type[Enum]]]] = [
    ("VertexToolName", TOOLS_ENUMS.VertexToolName, THEIR_ENUMS.VertexToolName),
    ("TaskTypeEnum", EMBEDDINGS_ENUMS.TaskTypeEnum, THEIR_ENUMS.TaskTypeEnum),
    (
        "VertexPartnerProvider",
        CREDENTIALS_ENUMS.VertexPartnerProvider,
        THEIR_ENUMS.VertexPartnerProvider,
    ),
]

GEMMA_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    {"object": "error", "message": "container failed", "code": 400},
    {"object": "error", "message": "", "code": 500},
    {"object": "error", "message": "m", "code": 599},
]


def shape(annotation: type) -> str:
    origin: type | None = get_origin(annotation)
    arguments: tuple[type, ...] = get_args(annotation)
    if is_typeddict(annotation):
        return f"TypedDict:{annotation.__name__}"
    if origin is None:
        if annotation.__name__ in ("Any", "JsonValue"):
            return "json"
        return annotation.__name__
    name = origin.__name__
    if name == "Literal":
        return f"Literal{list(arguments)}"
    if name in ("Union", "UnionType"):
        return "|".join(sorted(shape(argument) for argument in arguments))
    if name == "dict" and (not arguments or arguments[1].__name__ in ("Any", "JsonValue")):
        return "dict"
    return name + "[" + ",".join(shape(argument) for argument in arguments) + "]"


def signature_text(resolver: type[VertexAccessTokenResolver]) -> str:
    signature = inspect.signature(resolver.__call__)
    return str(signature)


async def resolve_token(
    credentials: VERTEX_CREDENTIALS_TYPES | None,
    project_id: str | None,
    custom_llm_provider: Literal["vertex_ai", "vertex_ai_beta", "gemini"],
) -> tuple[str, str]:
    await asyncio.sleep(0)
    resolved: str = "token"
    if credentials is not None:
        resolved = custom_llm_provider
    return (resolved, project_id if project_id is not None else "")


def listed_names() -> set[str]:
    names = {name for name, _, _ in TYPED_DICT_PAIRS}
    names.update(name for name, _, _ in ALIAS_PAIRS)
    names.update(name for name, _, _ in ENUM_PAIRS)
    names.add("VERTEX_AI_PROVIDER_METADATA_FIELDS")
    names.add("VertexAccessTokenResolver")
    return names


def test_names_cover_the_module() -> None:
    assert listed_names() == set(LITELLM_NAMES)
    assert set(dir(THEIRS)) >= LITELLM_NAMES


@pytest.mark.parametrize(("name", "ours", "theirs"), TYPED_DICT_PAIRS)
def test_names_typed_dict(name: str, ours: TypedDictClass, theirs: TypedDictClass) -> None:
    assert ours.__required_keys__ == theirs.__required_keys__, name
    assert ours.__optional_keys__ == theirs.__optional_keys__, name
    assert ours.__total__ == theirs.__total__, name
    our_hints: dict[str, type] = get_type_hints(ours)
    their_hints: dict[str, type] = get_type_hints(theirs)
    assert list(our_hints) == list(their_hints), name
    for key, hint in their_hints.items():
        assert shape(our_hints[key]) == shape(hint), f"{name}.{key}"


@pytest.mark.parametrize(("name", "ours", "theirs"), ALIAS_PAIRS)
def test_names_literal_values(
    name: str,
    ours: TypeForm[JsonValue],
    theirs: TypeForm[JsonValue],
) -> None:
    our_schema: dict[str, JsonValue] = TypeAdapter(ours).json_schema()
    their_schema: dict[str, JsonValue] = TypeAdapter(theirs).json_schema()
    assert our_schema == their_schema, name


@pytest.mark.parametrize(("name", "ours", "theirs"), ENUM_PAIRS)
def test_names_enum_members(name: str, ours: type[Enum], theirs: type[Enum]) -> None:
    our_members = [(member.name, member.value) for member in ours]
    their_members = [(member.name, member.value) for member in theirs]
    assert our_members == their_members, name


def test_names_provider_metadata_fields() -> None:
    theirs: tuple[str, ...] = THEIR_TUPLES.VERTEX_AI_PROVIDER_METADATA_FIELDS
    assert VERTEX_AI_PROVIDER_METADATA_FIELDS == theirs


def test_names_json_fields() -> None:
    tool_hints: dict[str, type] = get_type_hints(TOOLS.Tools)
    members: tuple[type, ...] = get_args(tool_hints["googleSearch"])
    assert members[0] is str
    assert members[1].__name__ == "JsonValue"
    schema_hints: dict[str, type] = get_type_hints(SCHEMA.Schema)
    assert schema_hints["default"].__name__ == "JsonValue"
    assert schema_hints["example"].__name__ == "JsonValue"


def test_protocol_resolver_signature() -> None:
    assert signature_text(VertexAccessTokenResolver) == signature_text(
        THEIR_RESOLVERS.VertexAccessTokenResolver,
    )


def test_protocol_resolver_async() -> None:
    assert inspect.iscoroutinefunction(VertexAccessTokenResolver.__call__)
    assert inspect.iscoroutinefunction(THEIR_RESOLVERS.VertexAccessTokenResolver.__call__)


def test_protocol_resolver_accepts_callable() -> None:
    resolver: VertexAccessTokenResolver = resolve_token
    token, project = asyncio.run(resolver({"token": "secret"}, "my-project", "gemini"))
    fallback, empty_project = asyncio.run(resolver(None, None, "vertex_ai"))
    assert (token, project) == ("gemini", "my-project")
    assert (fallback, empty_project) == ("token", "")


def test_protocol_gemma_fields() -> None:
    our_fields = VertexGemmaContainerError.model_fields
    their_fields = THEIR_GEMMA.VertexGemmaContainerError.model_fields
    assert list(our_fields) == list(their_fields)
    assert [field.is_required() for field in our_fields.values()] == [
        field.is_required() for field in their_fields.values()
    ]
    assert dict(VertexGemmaContainerError.model_config) == dict(
        THEIR_GEMMA.VertexGemmaContainerError.model_config,
    )


@pytest.mark.parametrize("payload", GEMMA_PAYLOADS)
def test_protocol_gemma_matches(payload: dict[str, JsonValue]) -> None:
    ours = VertexGemmaContainerError.model_validate(payload)
    theirs = THEIR_GEMMA.VertexGemmaContainerError.model_validate(payload)
    our_dump: dict[str, JsonValue] = ours.model_dump()
    their_dump: dict[str, JsonValue] = theirs.model_dump()
    assert our_dump == their_dump
    assert ours.message == their_dump["message"]
    assert ours.code == their_dump["code"]
    assert ours.object == "error"


@pytest.mark.parametrize("code", [399, 600])
def test_protocol_gemma_rejects_code_outside_bounds(code: int) -> None:
    payload: dict[str, JsonValue] = {"object": "error", "message": "m", "code": code}
    with pytest.raises(ValidationError):
        VertexGemmaContainerError.model_validate(payload)
    with pytest.raises(ValidationError):
        THEIR_GEMMA.VertexGemmaContainerError.model_validate(payload)


def test_protocol_gemma_frozen() -> None:
    error = VertexGemmaContainerError(object="error", message="m", code=400)
    with pytest.raises(ValidationError):
        error.code = 401
    with pytest.raises(ValidationError):
        error.message = "other"
