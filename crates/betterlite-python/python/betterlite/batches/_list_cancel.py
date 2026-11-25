import json
import logging
from collections.abc import Awaitable, Mapping
from typing import Final, Literal, TypeAlias

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall, acall_native, call_native
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeBatchListError, NativeBatchListSession
from betterlite._provider.lookup import get_llm_provider
from betterlite.batches._create_retrieve import (
    SETTING_NAMES,
    LiteLLMBatch,
    batch_from,
    param_value,
)
from betterlite.types.utils.base import ResponseObject

LOGGER: Final = logging.getLogger("LiteLLM")

ListBatchesSupportedProvider: TypeAlias = Literal[
    "openai", "azure", "hosted_vllm", "litellm_proxy", "vertex_ai", "xai",
]
CancelSupportedProvider: TypeAlias = Literal[
    "openai", "azure", "vertex_ai", "bedrock", "litellm_proxy", "xai",
]

# the GenericLiteLLMParams fields list_batches reads, under the names the Rust input takes
LIST_OPTIONAL: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "azure_ad_token", "azure_password", "azure_scope",
    "azure_username", "client_id", "client_secret", "max_retries", "organization",
    "request_timeout", "tenant_id", "timeout", "vertex_credentials", "vertex_location",
    "vertex_project",
)
# the CancelParams fields cancel_batch reads, the AWS overrides included
CANCEL_OPTIONAL: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "aws_access_key_id", "aws_region_name",
    "aws_secret_access_key", "aws_session_token", "azure_ad_token", "max_retries",
    "organization", "request_timeout", "timeout", "vertex_credentials", "vertex_location",
    "vertex_project",
)
# the keys LiteLLM's handlers take and the Rust payloads never carry
DROPPED: Final[tuple[str, ...]] = ("client", "logger_fn", "shared_session")


class BatchListPage(ResponseObject):
    data: list[LiteLLMBatch]
    has_more: bool | None = None


PAGE: Final[TypeAdapter[BatchListPage]] = TypeAdapter(BatchListPage)


def listing_session() -> NativeBatchListSession:
    settings: dict[str, JsonValue] = {"settings": read_settings(SETTING_NAMES)}
    return NativeBatchListSession(json.dumps(settings))


def list_values(
    after: str | None,
    limit: int | None,
    provider: str,
    options: Mapping[str, CallValue],
) -> CallPayload:
    values: dict[str, CallValue] = {"provider": provider, "after": after, "limit": limit}
    for name in LIST_OPTIONAL:
        values[name] = param_value(options, name)
    for name in DROPPED:
        values[name] = param_value(options, name)
    return to_call_payload(values)


def cancel_values(
    batch_id: str,
    model: str | None,
    provider: str,
    options: Mapping[str, CallValue],
) -> CallPayload:
    params: dict[str, CallValue] = {
        name: param_value(options, name) for name in CANCEL_OPTIONAL
    }
    call: dict[str, CallValue] = {
        "batch_id": batch_id,
        "model": model,
        "provider": provider,
        "extra_headers": param_value(options, "extra_headers"),
        "extra_body": param_value(options, "extra_body"),
        "params": params,
    }
    values: dict[str, CallValue] = {"cancel": call}
    for name in DROPPED:
        values[name] = param_value(options, name)
    return to_call_payload(values)


# the provider is inferred from the model before the branch, as LiteLLM's cancel_batch does
def cancel_provider(model: str | None, provider: str) -> str:
    if model is None:
        return provider
    try:
        _, inferred, _, _ = get_llm_provider(model, provider)
    except Exception:
        LOGGER.exception(
            "litellm.batches.main.py::cancel_batch() - Error inferring custom_llm_provider"
        )
        return provider
    return inferred


def batch_page(method: NativeCall, payload: CallPayload) -> BatchListPage:
    answer = call_native(method, payload, NativeBatchListError)
    return PAGE.validate_python(answer)


async def async_batch_page(method: AsyncNativeCall, payload: CallPayload) -> BatchListPage:
    answer = await acall_native(method, payload, NativeBatchListError)
    return PAGE.validate_python(answer)


def cancelled_batch(
    method: NativeCall, payload: CallPayload, kwargs: Mapping[str, CallValue]
) -> LiteLLMBatch:
    answer = call_native(method, payload, NativeBatchListError)
    return batch_from(answer, kwargs)


async def async_cancelled_batch(
    method: AsyncNativeCall, payload: CallPayload, kwargs: Mapping[str, CallValue]
) -> LiteLLMBatch:
    answer = await acall_native(method, payload, NativeBatchListError)
    return batch_from(answer, kwargs)


def list_batches(
    after: str | None = None,
    limit: int | None = None,
    custom_llm_provider: ListBatchesSupportedProvider = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> BatchListPage | Awaitable[BatchListPage]:
    if kwargs.pop("alist_batches", None) is True:
        return alist_batches(
            after, limit, custom_llm_provider, None, extra_headers, extra_body, **kwargs
        )
    options: dict[str, CallValue] = {
        **kwargs,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
    }
    payload = list_values(after, limit, custom_llm_provider, options)
    return batch_page(listing_session().list, payload)


async def alist_batches(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L700
    after: str | None = None,
    limit: int | None = None,
    custom_llm_provider: ListBatchesSupportedProvider = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> BatchListPage:
    # alist_batches declares metadata and list_batches does not; the source never passes it on
    options: dict[str, CallValue] = {
        **kwargs,
        "metadata": metadata,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
    }
    payload = list_values(after, limit, custom_llm_provider, options)
    return await async_batch_page(listing_session().alist, payload)


def cancel_batch(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L934
    batch_id: str,
    model: str | None = None,
    custom_llm_provider: CancelSupportedProvider | str = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch | Awaitable[LiteLLMBatch]:
    asynchronous = kwargs.pop("acancel_batch", None) is True
    provider = cancel_provider(model, custom_llm_provider)
    options: dict[str, CallValue] = {
        **kwargs,
        "metadata": metadata,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
    }
    payload = cancel_values(batch_id, model, provider, options)
    if asynchronous:
        return async_cancelled_batch(listing_session().acancel, payload, kwargs)
    return cancelled_batch(listing_session().cancel, payload, kwargs)


async def acancel_batch(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L888
    batch_id: str,
    model: str | None = None,
    custom_llm_provider: CancelSupportedProvider = "openai",
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> LiteLLMBatch:
    # LiteLLM's wrapper hands cancel_batch acancel_batch=True, then awaits the coroutine it answers
    kwargs["acancel_batch"] = True
    pending = cancel_batch(
        batch_id,
        model,
        custom_llm_provider,
        metadata,
        extra_headers,
        extra_body,
        **kwargs,
    )
    match pending:
        case LiteLLMBatch():
            return pending
        case Awaitable():
            return await pending
