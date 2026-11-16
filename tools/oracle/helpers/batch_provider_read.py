import os
from typing import Final, Literal, TypedDict

import httpx
import litellm
import litellm.batches.main as batches
from litellm.types.utils import LiteLLMBatch
from pydantic import BaseModel, Field, InstanceOf, JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
CAPTURED: list[tuple[str, dict[str, JsonValue | httpx.Timeout]]] = []


class RecordedError(Exception):
    pass


class UnknownRecordedProviderError(Exception):
    pass


class NoProviderReachedError(Exception):
    pass


class APIError(Exception):
    # the provider SDK raises an APIError that carries its status; the scripted provider imitates it
    def __init__(self, status: int) -> None:
        super().__init__(f"the scripted provider answered {status}")
        self.status_code = status


class Phases(BaseModel):
    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


class RetrieveRequest(TypedDict):
    batch_id: str
    extra_headers: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None


class AzureLitellmParams(TypedDict):
    api_key: str | None
    api_base: str | None
    api_version: str | None
    max_retries: int | None
    timeout: float | str | InstanceOf[httpx.Timeout] | None
    azure_ad_token: str | None
    tenant_id: str | None
    client_id: str | None
    client_secret: str | None
    azure_scope: str | None
    azure_username: str | None
    azure_password: str | None


RETRIEVE_REQUEST: Final = TypeAdapter(RetrieveRequest)
AZURE_LITELLM_PARAMS: Final = TypeAdapter(AzureLitellmParams)


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
    aws_session_name: str | None = None
    aws_profile_name: str | None = None
    aws_role_name: str | None = None
    aws_web_identity_token: str | None = None
    aws_sts_endpoint: str | None = None
    aws_external_id: str | None = None
    aws_session_tags: list[dict[str, str]] | None = None
    aws_bedrock_runtime_endpoint: str | None = None
    aws_bedrock_project_id: str | None = None


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
            raise APIError(self.status)
        raise RecordedError


def json_value(value: JsonValue | httpx.Timeout) -> JsonValue:
    match value:
        case httpx.Timeout() as timeout:
            return JSON.validate_python(timeout.as_dict())
        case str() | int() | float() | list() | dict() | None:
            return value


def request_value(value: JsonValue | httpx.Timeout) -> JsonValue:
    request = RETRIEVE_REQUEST.validate_python(value)
    return {
        "batch_id": json_value(request["batch_id"]),
        "extra_headers": json_value(request["extra_headers"]),
        "extra_body": json_value(request["extra_body"]),
    }


def azure_params(value: JsonValue | httpx.Timeout) -> JsonValue:
    params = AZURE_LITELLM_PARAMS.validate_python(value)
    return {
        "api_key": json_value(params["api_key"]),
        "api_base": json_value(params["api_base"]),
        "api_version": json_value(params["api_version"]),
        "max_retries": json_value(params["max_retries"]),
        "timeout": json_value(params["timeout"]),
        "azure_ad_token": json_value(params["azure_ad_token"]),
        "tenant_id": json_value(params["tenant_id"]),
        "client_id": json_value(params["client_id"]),
        "client_secret": json_value(params["client_secret"]),
        "azure_scope": json_value(params["azure_scope"]),
        "azure_username": json_value(params["azure_username"]),
        "azure_password": json_value(params["azure_password"]),
    }


def provider_timeout(value: float | str | Phases | None) -> float | str | httpx.Timeout | None:
    match value:
        case Phases() as phases:
            return httpx.Timeout(
                connect=phases.connect, read=phases.read, write=phases.write, pool=phases.pool
            )
        case int() | float() | str() | None:
            return value


CAPTURED_FIELDS: Final[dict[str, tuple[str, ...]]] = {
    "openai": ("api_base", "api_key", "organization", "timeout", "max_retries"),
    "azure": ("api_base", "api_key", "api_version", "timeout", "max_retries"),
    "vertex_ai": (
        "api_base",
        "vertex_project",
        "vertex_location",
        "vertex_credentials",
        "timeout",
        "max_retries",
    ),
    "anthropic": ("api_base", "api_key", "timeout", "max_retries"),
    "xai": ("api_base", "api_key", "timeout"),
}


def capture() -> JsonValue:
    provider, kwargs = CAPTURED[0]
    if provider not in CAPTURED_FIELDS:
        raise UnknownRecordedProviderError(provider)
    common: dict[str, JsonValue] = {
        "provider": provider,
        "is_async": json_value(kwargs["_is_async"]),
    }
    captured = common | {name: json_value(kwargs[name]) for name in CAPTURED_FIELDS[provider]}
    if "retrieve_batch_data" in kwargs:
        captured["request"] = request_value(kwargs["retrieve_batch_data"])
    else:
        captured["batch_id"] = json_value(kwargs["batch_id"])
    if "litellm_params" in kwargs:
        captured["litellm_params"] = azure_params(kwargs["litellm_params"])
    return captured


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
            aws_session_name=params.aws_session_name,
            aws_profile_name=params.aws_profile_name,
            aws_role_name=params.aws_role_name,
            aws_web_identity_token=params.aws_web_identity_token,
            aws_sts_endpoint=params.aws_sts_endpoint,
            aws_external_id=params.aws_external_id,
            aws_session_tags=params.aws_session_tags,
            aws_bedrock_runtime_endpoint=params.aws_bedrock_runtime_endpoint,
            aws_bedrock_project_id=params.aws_bedrock_project_id,
            model=request.model,
            aretrieve_batch=request.aretrieve_batch,
        )
    except RecordedError:
        return capture()
    raise NoProviderReachedError("the pinned LiteLLM retrieve call reached no provider")
