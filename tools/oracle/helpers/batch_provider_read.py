import os
from typing import Final, Literal, TypedDict

import httpx
import litellm
import litellm.batches.main as batches
from litellm.types.utils import LiteLLMBatch
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

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
CAPTURED: list[tuple[str, dict[str, JsonValue | httpx.Timeout]]] = []


class RecordedError(Exception):
    pass


class RecordedProviderError(Exception):
    def __init__(self, status: int) -> None:
        super().__init__("the scripted provider answered")
        self.status_code = status


class Phases(BaseModel):
    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


class OptionalParams(BaseModel):
    api_base: str | None = None
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    max_retries: int | None = None
    timeout: float | str | Phases | None = None
    request_timeout: float | None = None
    vertex_project: str | None = None
    vertex_location: str | None = None
    vertex_credentials: str | dict[str, JsonValue] | None = None
    azure_ad_token: str | None = None
    tenant_id: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    azure_scope: str | None = None
    azure_username: str | None = None
    azure_password: str | None = None
    aws_region_name: str | None = None
    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None


class Request(BaseModel):
    batch_id: str
    provider: Literal[
        "openai", "azure", "vertex_ai", "bedrock", "hosted_vllm", "litellm_proxy", "anthropic",
        "mistral", "xai",
    ] = "openai"
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    model: str | None = None
    aretrieve_batch: bool = False
    optional_params: OptionalParams = Field(default_factory=OptionalParams)


class Settings(BaseModel):
    api_key: str | None = None
    openai_key: str | None = None
    api_base: str | None = None
    organization: str | None = None
    azure_key: str | None = None
    api_version: str | None = None
    vertex_location: str | None = None
    vertex_project: str | None = None


class Script(BaseModel):
    status: int | None = None


class Input(TypedDict):
    request: dict[str, JsonValue]
    litellm_settings: dict[str, JsonValue]
    environment: dict[str, str]
    script: dict[str, JsonValue]


class Recorder:
    def __init__(self, provider: str, status: int | None) -> None:
        self.provider = provider
        self.status = status

    def retrieve_batch(self, **kwargs: JsonValue | httpx.Timeout) -> LiteLLMBatch:
        CAPTURED.append((self.provider, kwargs))
        if self.status is not None:
            raise RecordedProviderError(self.status)
        raise RecordedError


def plain(value: JsonValue | httpx.Timeout) -> JsonValue:
    if isinstance(value, httpx.Timeout):
        return JSON.validate_python(value.as_dict())
    return JSON.validate_python(value)


def request_value(value: JsonValue) -> JsonValue:
    if not isinstance(value, dict):
        return None
    return {
        "batch_id": plain(value.get("batch_id")),
        "extra_headers": plain(value.get("extra_headers")),
        "extra_body": plain(value.get("extra_body")),
    }


def azure_params(value: JsonValue | httpx.Timeout) -> JsonValue:
    if not isinstance(value, dict):
        return None
    return {name: plain(value.get(name)) for name in AZURE_PARAM_KEYS}


def provider_timeout(value: float | str | Phases | None) -> float | str | httpx.Timeout | None:
    if value is None or isinstance(value, str):
        return value
    if isinstance(value, Phases):
        return httpx.Timeout(
            connect=value.connect, read=value.read, write=value.write, pool=value.pool
        )
    return value


def capture() -> JsonValue:
    provider, kwargs = CAPTURED[0]
    if provider == "openai":
        return {
            "provider": provider,
            "is_async": plain(kwargs["_is_async"]),
            "request": request_value(plain(kwargs["retrieve_batch_data"])),
            "api_base": plain(kwargs["api_base"]),
            "api_key": plain(kwargs["api_key"]),
            "organization": plain(kwargs["organization"]),
            "timeout": plain(kwargs["timeout"]),
            "max_retries": plain(kwargs["max_retries"]),
        }
    if provider == "azure":
        return {
            "provider": provider,
            "is_async": plain(kwargs["_is_async"]),
            "request": request_value(plain(kwargs["retrieve_batch_data"])),
            "api_base": plain(kwargs["api_base"]),
            "api_key": plain(kwargs["api_key"]),
            "api_version": plain(kwargs["api_version"]),
            "timeout": plain(kwargs["timeout"]),
            "max_retries": plain(kwargs["max_retries"]),
            "litellm_params": azure_params(kwargs["litellm_params"]),
        }
    if provider == "vertex_ai":
        return {
            "provider": provider,
            "is_async": plain(kwargs["_is_async"]),
            "batch_id": plain(kwargs["batch_id"]),
            "api_base": plain(kwargs["api_base"]),
            "vertex_project": plain(kwargs["vertex_project"]),
            "vertex_location": plain(kwargs["vertex_location"]),
            "vertex_credentials": plain(kwargs["vertex_credentials"]),
            "timeout": plain(kwargs["timeout"]),
            "max_retries": plain(kwargs["max_retries"]),
        }
    if provider == "anthropic":
        return {
            "provider": provider,
            "is_async": plain(kwargs["_is_async"]),
            "batch_id": plain(kwargs["batch_id"]),
            "api_base": plain(kwargs["api_base"]),
            "api_key": plain(kwargs["api_key"]),
            "timeout": plain(kwargs["timeout"]),
            "max_retries": plain(kwargs["max_retries"]),
        }
    if provider == "xai":
        return {
            "provider": provider,
            "is_async": plain(kwargs["_is_async"]),
            "batch_id": plain(kwargs["batch_id"]),
            "api_base": plain(kwargs["api_base"]),
            "api_key": plain(kwargs["api_key"]),
            "timeout": plain(kwargs["timeout"]),
        }
    raise RuntimeError(f"the pinned LiteLLM retrieve reached the unknown provider {provider}")


def run(input_value: Input) -> JsonValue:
    request = Request.model_validate(input_value["request"])
    settings = Settings.model_validate(input_value["litellm_settings"])
    script = Script.model_validate(input_value["script"])
    params = request.optional_params
    for name, value in input_value["environment"].items():
        os.environ[name] = value
    litellm.api_key = settings.api_key
    litellm.openai_key = settings.openai_key
    litellm.api_base = settings.api_base
    object.__setattr__(litellm, "organization", settings.organization)
    litellm.azure_key = settings.azure_key
    litellm.api_version = settings.api_version
    litellm.vertex_location = settings.vertex_location
    litellm.vertex_project = settings.vertex_project
    object.__setattr__(batches, "openai_batches_instance", Recorder("openai", script.status))
    object.__setattr__(batches, "azure_batches_instance", Recorder("azure", script.status))
    object.__setattr__(batches, "vertex_ai_batches_instance", Recorder("vertex_ai", script.status))
    object.__setattr__(batches, "anthropic_batches_instance", Recorder("anthropic", script.status))
    object.__setattr__(batches, "xai_batches_instance", Recorder("xai", script.status))
    try:
        batches.retrieve_batch(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L560
            batch_id=request.batch_id,
            custom_llm_provider=request.provider,
            extra_headers=request.extra_headers,
            extra_body=request.extra_body,
            api_key=params.api_key,
            api_base=params.api_base,
            api_version=params.api_version,
            organization=params.organization,
            max_retries=params.max_retries,
            timeout=provider_timeout(params.timeout),
            request_timeout=params.request_timeout,
            vertex_project=params.vertex_project,
            vertex_location=params.vertex_location,
            vertex_credentials=params.vertex_credentials,
            azure_ad_token=params.azure_ad_token,
            tenant_id=params.tenant_id,
            client_id=params.client_id,
            client_secret=params.client_secret,
            azure_scope=params.azure_scope,
            azure_username=params.azure_username,
            azure_password=params.azure_password,
            aws_region_name=params.aws_region_name,
            aws_access_key_id=params.aws_access_key_id,
            aws_secret_access_key=params.aws_secret_access_key,
            aws_session_token=params.aws_session_token,
            model=request.model,
            aretrieve_batch=request.aretrieve_batch,
        )
    except RecordedError:
        return capture()
    raise RuntimeError("the pinned LiteLLM retrieve call reached no provider")
