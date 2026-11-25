from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import CallPayload, read_settings, to_call_payload
from betterlite._endpoints.payload import is_dropped

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HYPERPARAMETERS_DEFAULT: Final[dict[str, JsonValue]] = {}
SETTINGS: Final[tuple[str, ...]] = (
    "api_base",
    "api_key",
    "api_version",
    "azure_key",
    "openai_key",
    "organization",
    "vertex_location",
    "vertex_project",
)
# The kwargs each call reads through GenericLiteLLMParams, which its core params object accepts.
CREATE_PARAMS: Final[tuple[str, ...]] = (
    "api_base",
    "api_key",
    "api_version",
    "max_retries",
    "organization",
    "request_timeout",
    "timeout",
    "trainingType",
    "vertex_credentials",
    "vertex_location",
    "vertex_project",
)
READ_PARAMS: Final[tuple[str, ...]] = (
    "api_base",
    "api_key",
    "api_version",
    "max_retries",
    "organization",
    "request_timeout",
    "timeout",
)
# cancel reads the same kwargs plus the token GenericLiteLLMParams accepts and never uses
CANCEL_PARAMS: Final[tuple[str, ...]] = (
    "azure_ad_token",
    *READ_PARAMS,
)


def configured_params(
    kwargs: dict[str, JsonValue], names: tuple[str, ...]
) -> dict[str, JsonValue]:
    # a caller's client or shared session is refused here, the rule every endpoint family shares
    return {
        name: value
        for name, value in kwargs.items()
        if not is_dropped(name, value) and name in names
    }


def create_payload(
    model: str,
    training_file: str,
    hyperparameters: dict[str, JsonValue] | None,
    suffix: str | None,
    validation_file: str | None,
    integrations: list[str] | None,
    seed: int | None,
    custom_llm_provider: str,
    extra_headers: dict[str, str] | None,
    extra_body: dict[str, JsonValue] | None,
    kwargs: dict[str, JsonValue],
    *,
    is_async: bool,
) -> CallPayload:
    # the default hyperparameters dict is shared, so the record takes a copy and never the default
    source: dict[str, JsonValue] = {} if hyperparameters is None else hyperparameters
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
        "params": configured_params(kwargs, CREATE_PARAMS),
        "is_async": is_async,
    }
    return to_call_payload({"call": call, "settings": read_settings(SETTINGS)})


# The read calls differ only in their operation name and the fields that operation carries.
def read_payload(
    operation: str,
    fields: dict[str, JsonValue],
    kwargs: dict[str, JsonValue],
    *,
    is_async: bool,
) -> CallPayload:
    variant = f"a{operation}" if is_async else operation
    record: dict[str, JsonValue] = {variant: {**fields, **configured_params(kwargs, READ_PARAMS)}}
    return to_call_payload({"input": record, "settings": read_settings(SETTINGS)})


def cancel_payload(
    fine_tuning_job_id: str,
    custom_llm_provider: str,
    extra_headers: dict[str, str] | None,
    extra_body: dict[str, JsonValue] | None,
    kwargs: dict[str, JsonValue],
    *,
    is_async: bool,
) -> CallPayload:
    call: dict[str, JsonValue] = {
        "fine_tuning_job_id": fine_tuning_job_id,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "extra_body": extra_body,
        "acancel_fine_tuning_job": is_async,
        "params": configured_params(kwargs, CANCEL_PARAMS),
    }
    return to_call_payload({"call": call, "settings": read_settings(SETTINGS)})
