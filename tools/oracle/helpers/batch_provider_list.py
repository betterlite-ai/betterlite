import os
from typing import Final, Literal, TypedDict

import httpx
import litellm
import litellm.batches.main as batches
from litellm.llms.azure.batches.handler import AzureBatchesAPI
from litellm.llms.openai.openai import OpenAIBatchesAPI
from litellm.llms.vertex_ai.batches.handler import VertexAIBatchPrediction
from litellm.llms.xai.batches.handler import XAIBatchesHandler
from litellm.types.utils import LiteLLMBatch
from pydantic import BaseModel, JsonValue, TypeAdapter

AZURE_PARAM_KEYS: Final = (
    "api_key",
    "api_base",
    "api_version",
    "max_retries",
    "timeout",
    "azure_ad_token",
    "tenant_id",
    "client_id",
    "client_secret",
    "azure_scope",
    "azure_username",
    "azure_password",
)
JSON: Final = TypeAdapter(JsonValue)
CAPTURED: list[dict[str, JsonValue | httpx.Timeout]] = []


class RecordedError(Exception):
    pass


class Phases(BaseModel):
    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


class Request(BaseModel):
    provider: Literal[
        "openai", "azure", "hosted_vllm", "litellm_proxy", "vertex_ai", "xai"
    ] = "openai"
    after: str | None = None
    limit: int | None = None
    alist_batches: bool = False
    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | Phases | None = None
    request_timeout: float | None = None
    max_retries: int | None = None
    vertex_project: str | None = None
    vertex_location: str | None = None
    vertex_credentials: str | None = None
    azure_ad_token: str | None = None
    tenant_id: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    azure_scope: str | None = None
    azure_username: str | None = None
    azure_password: str | None = None


class Settings(BaseModel):
    api_key: str | None = None
    openai_key: str | None = None
    api_base: str | None = None
    organization: str | None = None
    api_version: str | None = None
    azure_key: str | None = None
    vertex_project: str | None = None
    vertex_location: str | None = None


class Input(TypedDict):
    request: dict[str, JsonValue]
    litellm_settings: dict[str, JsonValue]
    environment: dict[str, str]


def record_openai(_self: OpenAIBatchesAPI, **kwargs: JsonValue | httpx.Timeout) -> LiteLLMBatch:
    CAPTURED.append({"provider": "openai", **kwargs})
    raise RecordedError


def record_azure(_self: AzureBatchesAPI, **kwargs: JsonValue | httpx.Timeout) -> LiteLLMBatch:
    CAPTURED.append({"provider": "azure", **kwargs})
    raise RecordedError


def record_vertex(
    _self: VertexAIBatchPrediction, **kwargs: JsonValue | httpx.Timeout
) -> LiteLLMBatch:
    CAPTURED.append({"provider": "vertex_ai", **kwargs})
    raise RecordedError


def record_xai(_self: XAIBatchesHandler, **kwargs: JsonValue | httpx.Timeout) -> LiteLLMBatch:
    CAPTURED.append({"provider": "xai", **kwargs})
    raise RecordedError


def json_value(value: JsonValue | httpx.Timeout) -> JsonValue:
    if value is None:
        return None
    match value:
        case httpx.Timeout() as timeout:
            return JSON.validate_python(timeout.as_dict())
        case bool() as flag:
            return flag
        case int() | float() | str():
            return value
        case list() as items:
            return [json_value(item) for item in items]
        case dict() as fields:
            return {name: json_value(item) for name, item in fields.items()}


def azure_params(value: JsonValue) -> JsonValue:
    if value is None:
        return None
    match value:
        case dict() as fields:
            return {name: fields[name] for name in AZURE_PARAM_KEYS if name in fields}
        case bool() as flag:
            return flag
        case int() | float() | str() | list():
            return None


def provider_timeout(value: float | str | Phases | None) -> float | str | httpx.Timeout | None:
    if value is None:
        return None
    match value:
        case Phases() as phases:
            return httpx.Timeout(
                connect=phases.connect, read=phases.read, write=phases.write, pool=phases.pool
            )
        case int() | float() as seconds:
            return seconds
        case str() as text:
            return text


def capture() -> JsonValue:
    recorded = CAPTURED[0]
    values: dict[str, JsonValue] = {
        name: json_value(value) for name, value in recorded.items() if name != "_is_async"
    }
    values["is_async"] = json_value(recorded["_is_async"])
    if "litellm_params" in values:
        values["litellm_params"] = azure_params(values["litellm_params"])
    return values


def run(input_value: Input) -> JsonValue:
    request = Request.model_validate(input_value["request"])
    settings = Settings.model_validate(input_value["litellm_settings"])
    for name, value in input_value["environment"].items():
        os.environ[name] = value
    litellm.api_key = settings.api_key
    litellm.openai_key = settings.openai_key
    litellm.api_base = settings.api_base
    litellm.organization = settings.organization
    litellm.api_version = settings.api_version
    litellm.azure_key = settings.azure_key
    litellm.vertex_project = settings.vertex_project
    litellm.vertex_location = settings.vertex_location
    for target, record in (
        (OpenAIBatchesAPI, record_openai),
        (AzureBatchesAPI, record_azure),
        (VertexAIBatchPrediction, record_vertex),
        (XAIBatchesHandler, record_xai),
    ):
        type.__setattr__(target, "list_batches", record)
    try:
        batches.list_batches(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674
            after=request.after,
            limit=request.limit,
            custom_llm_provider=request.provider,
            alist_batches=request.alist_batches,
            api_key=request.api_key,
            api_base=request.api_base,
            api_version=request.api_version,
            organization=request.organization,
            timeout=provider_timeout(request.timeout),
            request_timeout=request.request_timeout,
            max_retries=request.max_retries,
            vertex_project=request.vertex_project,
            vertex_location=request.vertex_location,
            vertex_credentials=request.vertex_credentials,
            azure_ad_token=request.azure_ad_token,
            tenant_id=request.tenant_id,
            client_id=request.client_id,
            client_secret=request.client_secret,
            azure_scope=request.azure_scope,
            azure_username=request.azure_username,
            azure_password=request.azure_password,
        )
    except RecordedError:
        return capture()
    raise RuntimeError("the pinned LiteLLM list call reached no provider")
