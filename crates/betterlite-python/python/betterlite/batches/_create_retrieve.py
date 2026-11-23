import json
from collections.abc import Awaitable, Mapping
from typing import Final, Literal, TypeAlias

from pydantic import JsonValue, PrivateAttr, TypeAdapter
from typing_extensions import TypedDict, override

from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall, acall_native, call_native
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeBatchError, NativeBatchSession, NativeError
from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.usage import Usage

# call_native types its error argument as NativeError, so the class joins that family.
NativeBatchError.__bases__ = (NativeError,)

BatchPath: TypeAlias = Literal[
    "/v1/chat/completions", "/v1/embeddings", "/v1/completions", "/v1/responses", "/v1/ocr",
    "/v1/images/generations", "/v1/images/edits", "/v1/videos/generations",
    "/v1/videos", "/v1/videos/edits", "/v1/videos/extensions",
]
BatchState: TypeAlias = Literal[
    "validating", "failed", "in_progress", "finalizing",
    "completed", "expired", "cancelling", "cancelled",
]
CreateProvider: TypeAlias = Literal[
    "openai", "azure", "vertex_ai", "bedrock",
    "hosted_vllm", "litellm_proxy", "mistral", "xai",
]
ReadProvider: TypeAlias = Literal[
    "openai", "azure", "vertex_ai", "bedrock", "hosted_vllm",
    "litellm_proxy", "anthropic", "mistral", "xai",
]

# the GenericLiteLLMParams fields create_batch reads, under the names the Rust input takes
CREATE_OPTIONAL: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "organization", "timeout", "request_timeout",
    "max_retries", "vertex_project", "vertex_location", "vertex_credentials", "custom_endpoint",
)
# the GenericLiteLLMParams fields retrieve_batch reads, the AWS overrides included
READ_OPTIONAL: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "organization", "max_retries", "timeout",
    "request_timeout", "vertex_project", "vertex_location", "vertex_credentials",
    "azure_ad_token", "tenant_id", "client_id", "client_secret", "azure_scope",
    "azure_username", "azure_password", "aws_region_name", "aws_access_key_id",
    "aws_secret_access_key", "aws_session_token", "aws_session_name", "aws_profile_name",
    "aws_role_name", "aws_web_identity_token", "aws_sts_endpoint", "aws_external_id",
    "aws_session_tags", "aws_bedrock_runtime_endpoint", "aws_bedrock_project_id",
)
# the betterlite module globals the Rust session resolves its section settings from
SETTING_NAMES: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "azure_key", "openai_key", "organization",
    "vertex_location", "vertex_project",
)


class BatchModelInfo(TypedDict, total=False):
    id: str | None


MODEL_INFO: Final[TypeAdapter[BatchModelInfo | None]] = TypeAdapter(BatchModelInfo | None)
BATCH_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class LiteLLMBatch(ResponseObject):
    id: str
    completion_window: Literal["24h"]
    created_at: int
    endpoint: BatchPath
    input_file_id: str
    object: Literal["batch"]
    status: BatchState
    cancelled_at: int | None = None
    cancelling_at: int | None = None
    completed_at: int | None = None
    error_file_id: str | None = None
    errors: dict[str, JsonValue] | None = None
    expired_at: int | None = None
    expires_at: int | None = None
    failed_at: int | None = None
    finalizing_at: int | None = None
    in_progress_at: int | None = None
    metadata: dict[str, str] | None = None
    model: str | None = None
    output_file_id: str | None = None
    request_counts: dict[str, int] | None = None
    usage: Usage | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(self, **kwargs: JsonValue) -> None:
        raw_hidden = kwargs.pop("_hidden_params", None)
        hidden: dict[str, JsonValue] = {}
        if raw_hidden is not None:
            match raw_hidden:
                case Mapping():
                    hidden = dict(raw_hidden)
                case bool() | int() | float() | str() | list():
                    hidden = {}
        super().__init__(**kwargs)
        self._hidden_params = hidden


def param_value(kwargs: Mapping[str, CallValue], name: str) -> CallValue:
    if name in kwargs:
        return kwargs[name]
    return None


def batch_session() -> NativeBatchSession:
    settings: dict[str, JsonValue] = {"settings": read_settings(SETTING_NAMES)}
    return NativeBatchSession(json.dumps(settings))


def forwarded_values(
    kwargs: Mapping[str, CallValue], names: tuple[str, ...]
) -> dict[str, CallValue]:
    return {
        "model": param_value(kwargs, "model"),
        "optional_params": {name: param_value(kwargs, name) for name in names},
        "client": param_value(kwargs, "client"),
        "shared_session": param_value(kwargs, "shared_session"),
        "logger_fn": param_value(kwargs, "logger_fn"),
    }


def call_values(
    kwargs: Mapping[str, CallValue],
    names: tuple[str, ...],
    given: Mapping[str, CallValue],
) -> CallPayload:
    values = forwarded_values(kwargs, names)
    values.update(given)
    return to_call_payload(values)


def batch_from(answer: JsonValue, kwargs: Mapping[str, CallValue]) -> LiteLLMBatch:
    values = BATCH_OBJECT.validate_python(answer)
    hidden = BATCH_OBJECT.validate_python(values["_hidden_params"])
    info = MODEL_INFO.validate_python(param_value(kwargs, "model_info"))
    hidden["model_id"] = None if info is None else info.get("id")
    values["_hidden_params"] = hidden
    return LiteLLMBatch(**values)


def sync_answer(
    method: NativeCall, payload: CallPayload, kwargs: Mapping[str, CallValue]
) -> LiteLLMBatch:
    answer = call_native(method, payload, NativeBatchError)
    return batch_from(answer, kwargs)


async def async_answer(
    method: AsyncNativeCall, payload: CallPayload, kwargs: Mapping[str, CallValue]
) -> LiteLLMBatch:
    answer = await acall_native(method, payload, NativeBatchError)
    return batch_from(answer, kwargs)


def create_batch(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L172
    completion_window: Literal["24h"],
    endpoint: BatchPath,
    input_file_id: str,
    custom_llm_provider: CreateProvider = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    output_expires_after: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch | Awaitable[LiteLLMBatch]:
    if kwargs.pop("acreate_batch", None) is True:
        return acreate_batch(
            completion_window, endpoint, input_file_id, custom_llm_provider,
            metadata, extra_headers, extra_body, output_expires_after, **kwargs,
        )
    values: dict[str, CallValue] = {
        "completion_window": completion_window,
        "endpoint": endpoint,
        "input_file_id": input_file_id,
        "custom_llm_provider": custom_llm_provider,
        "metadata": metadata,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
        "output_expires_after": output_expires_after,
    }
    return sync_answer(
        batch_session().create, call_values(kwargs, CREATE_OPTIONAL, values), kwargs
    )


async def acreate_batch(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L108
    completion_window: Literal["24h"],
    endpoint: BatchPath,
    input_file_id: str,
    custom_llm_provider: CreateProvider = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    output_expires_after: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch:
    values: dict[str, CallValue] = {
        "completion_window": completion_window,
        "endpoint": endpoint,
        "input_file_id": input_file_id,
        "custom_llm_provider": custom_llm_provider,
        "metadata": metadata,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
        "output_expires_after": output_expires_after,
    }
    return await async_answer(
        batch_session().acreate, call_values(kwargs, CREATE_OPTIONAL, values), kwargs
    )


def retrieve_values(
    batch_id: str,
    custom_llm_provider: ReadProvider,
    extra_headers: dict[str, str] | None,
    extra_body: dict[str, str] | None,
    kwargs: Mapping[str, CallValue],
) -> CallPayload:
    values: dict[str, CallValue] = {
        "batch_id": batch_id,
        "provider": custom_llm_provider,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
    }
    return call_values(kwargs, READ_OPTIONAL, values)


def retrieve_batch(
    batch_id: str,
    custom_llm_provider: ReadProvider = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch | Awaitable[LiteLLMBatch]:
    if kwargs.pop("aretrieve_batch", None) is True:
        return aretrieve_batch(
            batch_id, custom_llm_provider, metadata, extra_headers, extra_body, **kwargs
        )
    return sync_answer(
        batch_session().retrieve,
        retrieve_values(batch_id, custom_llm_provider, extra_headers, extra_body, kwargs),
        kwargs,
    )


async def aretrieve_batch(
    batch_id: str,
    custom_llm_provider: ReadProvider = "openai",
    metadata: dict[str, str] | None = None,  # noqa: ARG001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L384
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch:
    payload = retrieve_values(batch_id, custom_llm_provider, extra_headers, extra_body, kwargs)
    return await async_answer(batch_session().aretrieve, payload, kwargs)
