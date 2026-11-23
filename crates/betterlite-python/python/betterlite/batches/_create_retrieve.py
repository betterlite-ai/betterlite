import json
from collections.abc import Awaitable, Mapping
from dataclasses import dataclass
from typing import Final, Literal, TypeAlias

from pydantic import JsonValue, PrivateAttr, TypeAdapter
from typing_extensions import override

from betterlite._endpoints.payload import CallValue, to_call_payload
from betterlite._endpoints.runner import acall_native, call_native
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeBatchError, NativeBatchSession
from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.usage import Usage

BatchPath: TypeAlias = Literal[
    "/v1/chat/completions",
    "/v1/embeddings",
    "/v1/completions",
    "/v1/responses",
    "/v1/ocr",
    "/v1/images/generations",
    "/v1/images/edits",
    "/v1/videos/generations",
    "/v1/videos",
    "/v1/videos/edits",
    "/v1/videos/extensions",
]
BatchState: TypeAlias = Literal[
    "validating",
    "failed",
    "in_progress",
    "finalizing",
    "completed",
    "expired",
    "cancelling",
    "cancelled",
]
CreateProvider: TypeAlias = Literal[
    "openai",
    "azure",
    "vertex_ai",
    "bedrock",
    "hosted_vllm",
    "litellm_proxy",
    "mistral",
    "xai",
]
ReadProvider: TypeAlias = Literal[
    "openai",
    "azure",
    "vertex_ai",
    "bedrock",
    "hosted_vllm",
    "litellm_proxy",
    "anthropic",
    "mistral",
    "xai",
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

BATCH_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


@dataclass(frozen=True, slots=True)
class CreateArguments:
    completion_window: Literal["24h"]
    endpoint: BatchPath
    input_file_id: str
    custom_llm_provider: CreateProvider
    metadata: dict[str, str] | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    output_expires_after: dict[str, JsonValue] | None


@dataclass(frozen=True, slots=True)
class RetrieveArguments:
    batch_id: str
    custom_llm_provider: ReadProvider
    metadata: dict[str, str] | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None


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


def param_values(
    kwargs: Mapping[str, CallValue], names: tuple[str, ...]
) -> dict[str, CallValue]:
    return {name: param_value(kwargs, name) for name in names}


def session_settings() -> str:
    return json.dumps({"settings": read_settings(SETTING_NAMES)})


def create_values(
    arguments: CreateArguments, kwargs: Mapping[str, CallValue]
) -> dict[str, CallValue]:
    return {
        "completion_window": arguments.completion_window,
        "endpoint": arguments.endpoint,
        "input_file_id": arguments.input_file_id,
        "custom_llm_provider": arguments.custom_llm_provider,
        "metadata": arguments.metadata,
        "extra_headers": arguments.extra_headers,
        "extra_body": arguments.extra_body,
        "output_expires_after": arguments.output_expires_after,
        "model": param_value(kwargs, "model"),
        "optional_params": param_values(kwargs, CREATE_OPTIONAL),
        "client": param_value(kwargs, "client"),
        "shared_session": param_value(kwargs, "shared_session"),
        "logger_fn": param_value(kwargs, "logger_fn"),
    }


def retrieve_values(
    arguments: RetrieveArguments, kwargs: Mapping[str, CallValue]
) -> dict[str, CallValue]:
    return {
        "batch_id": arguments.batch_id,
        "provider": arguments.custom_llm_provider,
        "extra_headers": arguments.extra_headers,
        "extra_body": arguments.extra_body,
        "model": param_value(kwargs, "model"),
        "optional_params": param_values(kwargs, READ_OPTIONAL),
        "client": param_value(kwargs, "client"),
        "shared_session": param_value(kwargs, "shared_session"),
        "logger_fn": param_value(kwargs, "logger_fn"),
    }


def batch_from(answer: JsonValue) -> LiteLLMBatch:
    values = BATCH_OBJECT.validate_python(answer)
    return LiteLLMBatch(**values)


def create_batch(
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
            completion_window,
            endpoint,
            input_file_id,
            custom_llm_provider,
            metadata,
            extra_headers,
            extra_body,
            output_expires_after,
            **kwargs,
        )
    arguments = CreateArguments(
        completion_window=completion_window,
        endpoint=endpoint,
        input_file_id=input_file_id,
        custom_llm_provider=custom_llm_provider,
        metadata=metadata,
        extra_headers=extra_headers,
        extra_body=extra_body,
        output_expires_after=output_expires_after,
    )
    payload = to_call_payload(create_values(arguments, kwargs))
    session = NativeBatchSession(session_settings())
    answer = call_native(session.create, payload, NativeBatchError)
    return batch_from(answer)


async def acreate_batch(
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
    arguments = CreateArguments(
        completion_window=completion_window,
        endpoint=endpoint,
        input_file_id=input_file_id,
        custom_llm_provider=custom_llm_provider,
        metadata=metadata,
        extra_headers=extra_headers,
        extra_body=extra_body,
        output_expires_after=output_expires_after,
    )
    payload = to_call_payload(create_values(arguments, kwargs))
    session = NativeBatchSession(session_settings())
    answer = await acall_native(session.acreate, payload, NativeBatchError)
    return batch_from(answer)


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
    arguments = RetrieveArguments(
        batch_id=batch_id,
        custom_llm_provider=custom_llm_provider,
        metadata=metadata,
        extra_headers=extra_headers,
        extra_body=extra_body,
    )
    payload = to_call_payload(retrieve_values(arguments, kwargs))
    session = NativeBatchSession(session_settings())
    answer = call_native(session.retrieve, payload, NativeBatchError)
    return batch_from(answer)


async def aretrieve_batch(
    batch_id: str,
    custom_llm_provider: ReadProvider = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch:
    arguments = RetrieveArguments(
        batch_id=batch_id,
        custom_llm_provider=custom_llm_provider,
        metadata=metadata,
        extra_headers=extra_headers,
        extra_body=extra_body,
    )
    payload = to_call_payload(retrieve_values(arguments, kwargs))
    session = NativeBatchSession(session_settings())
    answer = await acall_native(session.aretrieve, payload, NativeBatchError)
    return batch_from(answer)
