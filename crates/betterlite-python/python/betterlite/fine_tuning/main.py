from collections.abc import Awaitable
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import acall_native, call_native, read_settings, to_call_payload
from betterlite._native import NativeTuningError, NativeTuningSession
from betterlite.fine_tuning._calls import (
    configured_params,
    list_input,
    retrieve_input,
)
from betterlite.fine_tuning.types import FineTuningJobPage, page_value
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
HYPERPARAMETERS_DEFAULT: Final[dict[str, JsonValue]] = {}
SETTINGS: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "azure_key", "openai_key", "organization",
    "vertex_location", "vertex_project",
)


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
    source = {} if hyperparameters is None else hyperparameters
    call: dict[str, JsonValue] = {
        "model": model,
        "training_file": training_file,
        "hyperparameters": dict(source),
        "suffix": suffix,
        "validation_file": validation_file,
        "integrations": JSON.validate_python(integrations),
        "seed": seed,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "extra_body": extra_body,
        "params": configured_params(kwargs),
        "is_async": False,
    }
    payload = to_call_payload({"call": call, "settings": read_settings(SETTINGS)})
    job = call_native(NativeTuningSession().create, payload, NativeTuningError)
    return validated_value(job, LiteLLMFineTuningJob)


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
    source = {} if hyperparameters is None else hyperparameters
    call: dict[str, JsonValue] = {
        "model": model,
        "training_file": training_file,
        "hyperparameters": dict(source),
        "suffix": suffix,
        "validation_file": validation_file,
        "integrations": JSON.validate_python(integrations),
        "seed": seed,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "extra_body": extra_body,
        "params": configured_params(kwargs),
        "is_async": True,
    }
    payload = to_call_payload({"call": call, "settings": read_settings(SETTINGS)})
    job = await acall_native(NativeTuningSession().acreate, payload, NativeTuningError)
    return validated_value(job, LiteLLMFineTuningJob)


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
    call = retrieve_input(
        fine_tuning_job_id=fine_tuning_job_id,
        custom_llm_provider=custom_llm_provider,
        extra_headers=JSON.validate_python(extra_headers),
        extra_body=extra_body,
        params=configured_params(kwargs),
        is_async=False,
    )
    payload = to_call_payload({**call, "settings": read_settings(SETTINGS)})
    job = call_native(NativeTuningSession().retrieve, payload, NativeTuningError)
    return validated_value(job, LiteLLMFineTuningJob)


async def aretrieve_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob:
    call = retrieve_input(
        fine_tuning_job_id=fine_tuning_job_id,
        custom_llm_provider=custom_llm_provider,
        extra_headers=JSON.validate_python(extra_headers),
        extra_body=extra_body,
        params=configured_params(kwargs),
        is_async=True,
    )
    payload = to_call_payload({**call, "settings": read_settings(SETTINGS)})
    job = await acall_native(NativeTuningSession().aretrieve, payload, NativeTuningError)
    return validated_value(job, LiteLLMFineTuningJob)


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
    call = list_input(
        after=after,
        limit=limit,
        custom_llm_provider=custom_llm_provider,
        extra_headers=JSON.validate_python(extra_headers),
        extra_body=extra_body,
        params=configured_params(kwargs),
        is_async=False,
    )
    payload = to_call_payload({**call, "settings": read_settings(SETTINGS)})
    page = call_native(NativeTuningSession().list, payload, NativeTuningError)
    return page_value(page)


async def alist_fine_tuning_jobs(
    after: str | None = None,
    limit: int | None = None,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> FineTuningJobPage:
    call = list_input(
        after=after,
        limit=limit,
        custom_llm_provider=custom_llm_provider,
        extra_headers=JSON.validate_python(extra_headers),
        extra_body=extra_body,
        params=configured_params(kwargs),
        is_async=True,
    )
    payload = to_call_payload({**call, "settings": read_settings(SETTINGS)})
    page = await acall_native(NativeTuningSession().alist, payload, NativeTuningError)
    return page_value(page)


def cancel_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob]:
    if kwargs.pop("acancel_fine_tuning_job", False) is True:
        return acancel_fine_tuning_job(
            fine_tuning_job_id, custom_llm_provider, extra_headers, extra_body, **kwargs,
        )
    call: dict[str, JsonValue] = {
        "fine_tuning_job_id": fine_tuning_job_id,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "extra_body": extra_body,
        "acancel_fine_tuning_job": False,
        "params": configured_params(kwargs),
    }
    payload = to_call_payload({"call": call, "settings": read_settings(SETTINGS)})
    job = call_native(NativeTuningSession().cancel, payload, NativeTuningError)
    return validated_value(job, LiteLLMFineTuningJob)


async def acancel_fine_tuning_job(
    fine_tuning_job_id: str,
    custom_llm_provider: str = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: JsonValue,
) -> LiteLLMFineTuningJob:
    call: dict[str, JsonValue] = {
        "fine_tuning_job_id": fine_tuning_job_id,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "extra_body": extra_body,
        "acancel_fine_tuning_job": True,
        "params": configured_params(kwargs),
    }
    payload = to_call_payload({"call": call, "settings": read_settings(SETTINGS)})
    job = await acall_native(NativeTuningSession().acancel, payload, NativeTuningError)
    return validated_value(job, LiteLLMFineTuningJob)
