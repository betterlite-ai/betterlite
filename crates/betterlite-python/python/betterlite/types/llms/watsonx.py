from enum import StrEnum
from typing import NotRequired, TypedDict


class WatsonXAPIParams(TypedDict):
    project_id: str | None
    space_id: str | None
    region_name: str | None


class WatsonXCredentials(TypedDict):
    api_key: str
    api_base: str
    token: str | None


class WatsonXAudioTranscriptionRequestBody(TypedDict):  # ast-grep-ignore: no-long-names
    model: str
    project_id: NotRequired[str]
    space_id: NotRequired[str]
    language: NotRequired[str]
    prompt: NotRequired[str]
    response_format: NotRequired[str]
    temperature: NotRequired[float]
    timestamp_granularities: NotRequired[list[str]]


class WatsonXAIEndpoint(StrEnum):
    TEXT_GENERATION = "/ml/v1/text/generation"
    TEXT_GENERATION_STREAM = "/ml/v1/text/generation_stream"
    CHAT = "/ml/v1/text/chat"
    CHAT_STREAM = "/ml/v1/text/chat_stream"
    DEPLOYMENT_TEXT_GENERATION = "/ml/v1/deployments/{deployment_id}/text/generation"
    # ast-grep-ignore: no-long-names
    DEPLOYMENT_TEXT_GENERATION_STREAM = "/ml/v1/deployments/{deployment_id}/text/generation_stream"
    DEPLOYMENT_CHAT = "/ml/v1/deployments/{deployment_id}/text/chat"
    DEPLOYMENT_CHAT_STREAM = "/ml/v1/deployments/{deployment_id}/text/chat_stream"
    EMBEDDINGS = "/ml/v1/text/embeddings"
    PROMPTS = "/ml/v1/prompts"
    AVAILABLE_MODELS = "/ml/v1/foundation_model_specs"
    RERANK = "/ml/v1/text/rerank"


class WatsonXModelPattern(StrEnum):
    GRANITE_CHAT = "granite-chat"
    IBM_MISTRAL = "ibm-mistral"
    IBM_MISTRALAI = "ibm-mistralai"
    GPT_OSS = "openai/gpt-oss"
    LLAMA3_INSTRUCT = "meta-llama/llama-3-instruct"
