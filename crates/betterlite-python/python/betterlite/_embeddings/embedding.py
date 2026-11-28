import uuid
from collections.abc import Awaitable, Mapping
from typing import Final

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite import exceptions
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    model_from_arguments,
    proxy_headers,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeEmbeddingError, NativeEmbeddingSession
from betterlite.types.utils import Embedding, EmbeddingResponse

DEFAULT_TIMEOUT: Final = 600
MISSING_MODEL: Final = (
    "model parameter is required but was None. Please provide a valid model name."
)
# the module settings main.py's embedding reads before it dispatches, as LiteLLM's branches do
SETTINGS: Final[tuple[str, ...]] = ("api_base", "api_key", "drop_params", "openai_key")
# every argument embedding names itself: these never travel as an unknown provider parameter
NAMED_ARGUMENTS: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_type", "api_version", "caching", "custom_llm_provider",
    "dimensions", "encoding_format", "extra_headers", "headers", "input", "litellm_call_id",
    "logger_fn", "model", "timeout", "user",
)
# the named arguments Rust reads; a None one stays out so its default applies there
OPTIONAL_ARGUMENTS: Final[tuple[str, ...]] = (
    "dimensions", "encoding_format", "user", "api_base", "api_version", "api_key",
    "custom_llm_provider",
)
# LiteLLM's own bookkeeping and the clients Rust cannot use; the payload drops or refuses them
SKIPPED_KWARGS: Final[tuple[str, ...]] = (
    "aembedding", "azure_ad_token_provider", "client", "litellm_logging_obj", "shared_session",
)
# aembedding's positional arguments, in embedding's own order
POSITIONAL_ARGUMENTS: Final[tuple[str, ...]] = (
    "model", "input", "dimensions", "encoding_format", "timeout", "api_base", "api_version",
    "api_key", "api_type", "caching", "user", "custom_llm_provider", "litellm_call_id",
    "logger_fn",
)

TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HEADERS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])


class EmbeddingAnswer(TypedDict):
    object: str
    model: str
    data: list[JsonValue]
    usage: JsonValue
    hidden_params: dict[str, JsonValue]
    response_headers: dict[str, str]
    response_ms: float


ANSWER: Final[TypeAdapter[EmbeddingAnswer]] = TypeAdapter(EmbeddingAnswer)


def embedding(
    model: str | None,
    input: JsonValue = [],
    dimensions: int | None = None,
    encoding_format: str | None = None,
    timeout: float = 600,
    api_base: str | None = None,
    api_version: str | None = None,
    api_key: str | None = None,
    api_type: str | None = None,
    caching: bool = False,
    user: str | None = None,
    custom_llm_provider: str | None = None,
    litellm_call_id: str | None = None,
    logger_fn: CallValue = None,
    **kwargs: CallValue,
) -> EmbeddingResponse | Awaitable[EmbeddingResponse]:
    named: dict[str, CallValue] = {
        "model": model, "input": input, "dimensions": dimensions,
        "encoding_format": encoding_format, "timeout": timeout, "api_base": api_base,
        "api_version": api_version, "api_key": api_key, "api_type": api_type,
        "caching": caching, "user": user, "custom_llm_provider": custom_llm_provider,
        "litellm_call_id": litellm_call_id, "logger_fn": logger_fn,
    }
    return embedding_from_values({**kwargs, **named})


async def aembedding(*args: CallValue, **kwargs: CallValue) -> EmbeddingResponse:
    # LiteLLM reads the model before it awaits, so a missing one raises at the call, not in the task
    model = TEXT.validate_python(model_from_arguments(args, kwargs))
    if model is None:
        raise exceptions.BadRequestError(
            f"GetLLMProvider Exception - {MISSING_MODEL}\n\noriginal model: None", None, ""
        )
    positional = dict(zip(POSITIONAL_ARGUMENTS, args, strict=False))
    return await embedding_async({**kwargs, **positional})


def embedding_from_values(
    values: Mapping[str, CallValue],
) -> EmbeddingResponse | Awaitable[EmbeddingResponse]:
    model = TEXT.validate_python(values.get("model"))
    if model is None:
        raise exceptions.BadRequestError(
            f"GetLLMProvider Exception - {MISSING_MODEL}\n\noriginal model: None", None, ""
        )
    if values.get("aembedding") is True:
        return embedding_async(values)
    return embedding_sync(values)


def embedding_sync(values: Mapping[str, CallValue]) -> EmbeddingResponse:
    answer = call_native(
        NativeEmbeddingSession("{}").embedding, call_payload(values), NativeEmbeddingError
    )
    return response_from_answer(ANSWER.validate_python(answer))


async def embedding_async(values: Mapping[str, CallValue]) -> EmbeddingResponse:
    answer = await acall_native(
        NativeEmbeddingSession("{}").aembedding, call_payload(values), NativeEmbeddingError
    )
    return response_from_answer(ANSWER.validate_python(answer))


def response_from_answer(answer: EmbeddingAnswer) -> EmbeddingResponse:
    data = [Embedding.model_validate(item) for item in answer["data"]]
    return EmbeddingResponse(
        model=answer["model"],
        usage=answer["usage"],
        response_ms=answer["response_ms"],
        data=data,
        hidden_params=answer["hidden_params"],
        _response_headers=answer["response_headers"],
    )


def call_payload(values: Mapping[str, CallValue]) -> CallPayload:
    # the top-level names are the payload's own: it drops the logger, refuses a client and warns
    payload: dict[str, CallValue] = {
        "arguments": forwarded_arguments(values),
        "settings": read_settings(SETTINGS),
        "client": values.get("client"),
        "logger_fn": values.get("logger_fn"),
        "shared_session": values.get("shared_session"),
    }
    return to_call_payload(payload)


def forwarded_arguments(values: Mapping[str, CallValue]) -> dict[str, CallValue]:
    if values.get("azure_ad_token_provider") is not None:
        raise UnsupportedCallArgument("azure_ad_token_provider")
    model = TEXT.validate_python(values.get("model"))
    given_input = JSON.validate_python(values.get("input"))
    given_timeout = values.get("timeout")
    forwarded: dict[str, CallValue] = {
        "model": "" if model is None else model,
        "input": [] if given_input is None else given_input,
        "headers": merged_headers(values),
        "litellm_call_id": call_id(values.get("litellm_call_id")),
        "timeout": DEFAULT_TIMEOUT if given_timeout is None else given_timeout,
    }
    for name in OPTIONAL_ARGUMENTS:
        value = values.get(name)
        if value is not None:
            forwarded[name] = value
    forwarded |= extra_arguments(values)
    return forwarded


def extra_arguments(values: Mapping[str, CallValue]) -> dict[str, CallValue]:
    return {
        name: value
        for name, value in values.items()
        if value is not None and name not in NAMED_ARGUMENTS and name not in SKIPPED_KWARGS
    }


def merged_headers(values: Mapping[str, CallValue]) -> dict[str, str]:
    # LiteLLM's `headers or extra_headers`, then extra_headers and the proxy's own over it
    merged: dict[str, str] = {}
    for name in ("headers", "extra_headers"):
        given = values.get(name)
        if given is not None:
            merged |= HEADERS.validate_python(given)
    merged |= proxy_headers()
    return merged


def call_id(value: CallValue | None) -> str:
    text = TEXT.validate_python(value)
    if not text:
        return str(uuid.uuid4())
    return text
