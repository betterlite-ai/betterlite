from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING, Literal, Protocol, TypeAlias, cast

from litellm.litellm_core_utils.get_supported_openai_params import (
    get_supported_openai_params,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.litellm_core_utils.model_param_helper import ModelParamHelper
from litellm.litellm_core_utils.reasoning_effort_utils import (
    reasoning_effort_from_thinking_budget,
)
from litellm.litellm_core_utils.service_tier_utils import (
    get_service_tier_from_standard_logging_payload,
)
from litellm.utils import get_optional_params  # pyright: ignore[reportUnknownVariableType]

if TYPE_CHECKING:
    from collections.abc import Callable

    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
RequestType: TypeAlias = Literal["chat_completion", "embeddings", "transcription"]


class OptionalParamsCall(Protocol):
    def __call__(self, **kwargs: object) -> dict[str, object]: ...


_optional_params = cast(OptionalParamsCall, get_optional_params)
_standard_logging_parameters = cast(
    "Callable[[dict[str, object]], dict[str, object]]",
    ModelParamHelper.get_standard_logging_model_parameters,
)


def _object(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _bool(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


def _string_list(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [_string(item, name) for item in value]


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value


def _supported(step: Object) -> Json:
    query = _object(step.get("query"), "query")
    base_model = query.get("base_model")
    result = cast(
        "list[object] | None",
        get_supported_openai_params(
            model=_string(query.get("model"), "model"),
            custom_llm_provider=_string(query.get("provider"), "provider"),
            request_type=cast(
                RequestType, _string(query.get("call_type"), "call_type")
            ),
            base_model=None if base_model is None else _string(base_model, "base_model"),
        ),
    )
    return cast(Json, result)


def _mapping(step: Object) -> Json:
    query = _object(step.get("query"), "query")
    parameters = _object(step.get("parameters", {}), "parameters")
    policy = _object(step.get("policy", {}), "policy")
    kwargs: dict[str, object] = dict(parameters)
    kwargs.setdefault("stream", None)
    kwargs["model"] = _string(query.get("model"), "model")
    kwargs["custom_llm_provider"] = _string(query.get("provider"), "provider")
    kwargs["drop_params"] = _bool(policy.get("drop_params", False), "drop_params")
    kwargs["allowed_openai_params"] = _string_list(
        policy.get("allowed_openai_params", []), "allowed_openai_params"
    )
    kwargs["additional_drop_params"] = _string_list(
        policy.get("additional_drop_params", []), "additional_drop_params"
    )
    output = _optional_params(**kwargs)
    extra_body = output.pop("extra_body", {})
    provider_fields: dict[str, object] = (
        cast("dict[str, object]", extra_body) if isinstance(extra_body, dict) else {}
    )
    standard = {
        name: cast(Json, value) for name, value in output.items() if value is not None
    }
    provider = {
        name: cast(Json, value)
        for name, value in provider_fields.items()
        if value is not None
    }
    dropped = sorted(
        name
        for name, value in parameters.items()
        if value is not None
        and name != "extra_body"
        and name not in standard
        and name not in provider
    )
    return {
        "standard": cast(Json, standard),
        "provider_fields": cast(Json, provider),
        "dropped_fields": cast(Json, dropped),
    }


def _reasoning_effort(step: Object) -> Json:
    return reasoning_effort_from_thinking_budget(
        _integer(step.get("budget_tokens"), "budget_tokens")
    )


def _service_tier_payload(step: Object) -> StandardLoggingPayload:
    fields: dict[str, Json] = {}
    response_tier = step.get("response_tier")
    if response_tier is not None:
        fields["response"] = {"service_tier": _string(response_tier, "response_tier")}
    usage_tier = step.get("usage_tier")
    if usage_tier is not None:
        fields["metadata"] = {
            "usage_object": {"service_tier": _string(usage_tier, "usage_tier")}
        }
    requested_tier = step.get("requested_tier")
    if requested_tier is not None:
        fields["model_parameters"] = {
            "service_tier": _string(requested_tier, "requested_tier")
        }
    return cast("StandardLoggingPayload", cast(object, fields))


def _resolved_service_tier(step: Object) -> Json:
    return get_service_tier_from_standard_logging_payload(_service_tier_payload(step))


def _logging_projection(step: Object) -> Json:
    parameters = _object(step.get("model_parameters"), "model_parameters")
    return cast(
        Json,
        _standard_logging_parameters(cast("dict[str, object]", parameters)),
    )


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "supported_params": _supported,
    "map_parameters": _mapping,
    "reasoning_effort": _reasoning_effort,
    "resolved_service_tier": _resolved_service_tier,
    "logging_projection": _logging_projection,
}


def _run_step(step: Json) -> Json:
    record = _object(step, "step")
    function = _string(record.get("function"), "function")
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unsupported provider_params function {function}")
    return handler(record)


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    return [_run_step(step) for step in steps]
