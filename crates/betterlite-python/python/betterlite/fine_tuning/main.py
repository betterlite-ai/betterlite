from collections.abc import Awaitable
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import CallPayload, acall_native, call_native
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall
from betterlite._native import NativeTuningError, NativeTuningSession
from betterlite.fine_tuning._calls import (
    HYPERPARAMETERS_DEFAULT,
    cancel_payload,
    create_payload,
    read_payload,
)
from betterlite.fine_tuning._page import FineTuningJobPage
from betterlite.types.utils import LiteLLMFineTuningJob
from betterlite.types.utils.base import validated_value

__all__ = [
    "acancel_fine_tuning_job",
    "acreate_fine_tuning_job",
    "alist_fine_tuning_jobs",
    "aretrieve_fine_tuning_job",
    "cancel_fine_tuning_job",
    "create_fine_tuning_job",
    "list_fine_tuning_jobs",
    "retrieve_fine_tuning_job",
]

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def job_answer(method: NativeCall, payload: CallPayload) -> LiteLLMFineTuningJob:
    answer = call_native(method, payload, NativeTuningError)
    return validated_value(answer, LiteLLMFineTuningJob)


async def async_job_answer(method: AsyncNativeCall, payload: CallPayload) -> LiteLLMFineTuningJob:
    answer = await acall_native(method, payload, NativeTuningError)
    return validated_value(answer, LiteLLMFineTuningJob)


def create_fine_tuning_job(
    model: str,
    training_file: str,
    hyperparameters: dict[str, JsonValue] | None = HYPERPARAMETERS_DEFAULT,
    suffix: str | None = None,
    validation_file: str | None = None,
    integrations: list[str] | None = None,
    seed: int | None = None,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob]:
    if kwargs.pop("acreate_fine_tuning_job", False) is True:
        return acreate_fine_tuning_job(
            model, training_file, hyperparameters, suffix, validation_file, integrations,
            seed, custom_llm_provider, extra_headers, extra_body, **kwargs,
        )
    payload = create_payload(
        model, training_file, hyperparameters, suffix, validation_file, integrations,
        seed, custom_llm_provider, extra_headers, extra_body, kwargs, is_async=False,
    )
    return job_answer(NativeTuningSession().create, payload)


async def acreate_fine_tuning_job(
    model: str,
    training_file: str,
    hyperparameters: dict[str, JsonValue] | None = HYPERPARAMETERS_DEFAULT,
    suffix: str | None = None,
    validation_file: str | None = None,
    integrations: list[str] | None = None,
    seed: int | None = None,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob:
    payload = create_payload(
        model, training_file, hyperparameters, suffix, validation_file, integrations,
        seed, custom_llm_provider, extra_headers, extra_body, kwargs, is_async=True,
    )
    return await async_job_answer(NativeTuningSession().acreate, payload)


def retrieve_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob]:
    if kwargs.pop("aretrieve_fine_tuning_job", False) is True:
        return aretrieve_fine_tuning_job(
            fine_tuning_job_id, custom_llm_provider, extra_headers, extra_body, **kwargs,
        )
    payload = read_payload(
        "retrieve",
        {
            "fine_tuning_job_id": fine_tuning_job_id,
            "provider": custom_llm_provider,
            "extra_headers": JSON.validate_python(extra_headers),
            "extra_body": extra_body,
        },
        kwargs,
        is_async=False,
    )
    return job_answer(NativeTuningSession().retrieve, payload)


async def aretrieve_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob:
    fields: dict[str, JsonValue] = {
        "fine_tuning_job_id": fine_tuning_job_id,
        "provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "extra_body": extra_body,
    }
    payload = read_payload("retrieve", fields, kwargs, is_async=True)
    return await async_job_answer(NativeTuningSession().aretrieve, payload)


def list_fine_tuning_jobs(
    after: str | None = None,
    limit: int | None = None,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> FineTuningJobPage | Awaitable[FineTuningJobPage]:
    if kwargs.pop("alist_fine_tuning_jobs", False) is True:
        return alist_fine_tuning_jobs(
            after, limit, custom_llm_provider, extra_headers, extra_body, **kwargs,
        )
    page = read_payload(
        "list",
        {
            "after": after,
            "limit": limit,
            "provider": custom_llm_provider,
            "extra_headers": JSON.validate_python(extra_headers),
            "extra_body": extra_body,
        },
        kwargs,
        is_async=False,
    )
    answer = call_native(NativeTuningSession().list, page, NativeTuningError)
    return validated_value(answer, FineTuningJobPage)


async def alist_fine_tuning_jobs(
    after: str | None = None,
    limit: int | None = None,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> FineTuningJobPage:
    page = read_payload(
        "list",
        {
            "after": after,
            "limit": limit,
            "provider": custom_llm_provider,
            "extra_headers": JSON.validate_python(extra_headers),
            "extra_body": extra_body,
        },
        kwargs,
        is_async=True,
    )
    answer = await acall_native(NativeTuningSession().alist, page, NativeTuningError)
    return validated_value(answer, FineTuningJobPage)


def cancel_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob]:
    forwarded = kwargs.pop("acancel_fine_tuning_job", False)
    if forwarded is True:
        return acancel_fine_tuning_job(
            fine_tuning_job_id, custom_llm_provider, extra_headers, extra_body, **kwargs,
        )
    payload = cancel_payload(
        fine_tuning_job_id,
        custom_llm_provider,
        extra_headers,
        extra_body,
        kwargs,
        is_async=False,
    )
    return job_answer(NativeTuningSession().cancel, payload)


async def acancel_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob:
    payload = cancel_payload(
        fine_tuning_job_id,
        custom_llm_provider,
        extra_headers,
        extra_body,
        kwargs,
        is_async=True,
    )
    return await async_job_answer(NativeTuningSession().acancel, payload)
