from typing import Final

from pydantic import JsonValue

from betterlite._endpoints.payload import is_dropped

# The kwargs GenericLiteLLMParams reads, which the core call carries as its params object.
CONFIGURED_PARAMS: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "max_retries", "organization", "request_timeout",
    "timeout", "trainingType", "vertex_credentials", "vertex_location", "vertex_project",
)


def configured_params(kwargs: dict[str, JsonValue]) -> dict[str, JsonValue]:
    # a caller's client or shared session is refused here, the rule every endpoint family shares
    for name, value in kwargs.items():
        is_dropped(name, value)
    return {name: kwargs[name] for name in CONFIGURED_PARAMS if name in kwargs}


def list_input(
    after: str | None,
    limit: int | None,
    custom_llm_provider: str,
    extra_headers: JsonValue,
    extra_body: JsonValue,
    params: dict[str, JsonValue],
    *,
    is_async: bool,
) -> dict[str, JsonValue]:
    variant = "alist" if is_async else "list"
    return {
        "input": {
            variant: {
                "after": after,
                "limit": limit,
                "provider": custom_llm_provider,
                "extra_headers": extra_headers,
                "extra_body": extra_body,
                **params,
            }
        }
    }


def retrieve_input(
    fine_tuning_job_id: str,
    custom_llm_provider: str,
    extra_headers: JsonValue,
    extra_body: JsonValue,
    params: dict[str, JsonValue],
    *,
    is_async: bool,
) -> dict[str, JsonValue]:
    variant = "aretrieve" if is_async else "retrieve"
    return {
        "input": {
            variant: {
                "fine_tuning_job_id": fine_tuning_job_id,
                "provider": custom_llm_provider,
                "extra_headers": extra_headers,
                "extra_body": extra_body,
                **params,
            }
        }
    }
