from __future__ import annotations

from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.litellm_core_utils.model_param_helper import ModelParamHelper
from litellm.litellm_core_utils.reasoning_effort_utils import (
    reasoning_effort_from_thinking_budget,
)
from litellm.litellm_core_utils.service_tier_utils import (
    get_service_tier_from_standard_logging_payload,
)

if TYPE_CHECKING:
    from collections.abc import Callable

    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _optional_string(step: dict[str, Json], name: str) -> str | None:
    value = step.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _logging_projection(step: dict[str, Json]) -> Json:
    parameters = _object(step.get("model_parameters"), "model_parameters")
    standard = cast(
        "Callable[[dict[str, object]], dict[str, object]]",
        ModelParamHelper.get_standard_logging_model_parameters,
    )
    return cast(Json, standard(cast("dict[str, object]", parameters)))


def _service_tier_payload(step: dict[str, Json]) -> StandardLoggingPayload:
    response_tier = _optional_string(step, "response_tier")
    usage_tier = _optional_string(step, "usage_tier")
    requested_tier = _optional_string(step, "requested_tier")
    fields: dict[str, Json] = {}
    if response_tier is not None:
        fields["response"] = {"service_tier": response_tier}
    if usage_tier is not None:
        fields["metadata"] = {"usage_object": {"service_tier": usage_tier}}
    if requested_tier is not None:
        fields["model_parameters"] = {"service_tier": requested_tier}
    return cast("StandardLoggingPayload", cast(object, fields))


def _reasoning_effort(step: dict[str, Json]) -> Json:
    budget_tokens = step.get("budget_tokens")
    if not isinstance(budget_tokens, int) or isinstance(budget_tokens, bool):
        raise TypeError("budget_tokens must be an integer")
    return reasoning_effort_from_thinking_budget(budget_tokens)


def _run_step(step: dict[str, Json]) -> Json:
    function = step.get("function")
    if function == "logging_projection":
        return _logging_projection(step)
    if function == "excluded_content_names":
        excluded = ModelParamHelper.get_exclude_params_for_model_parameters()
        return cast(Json, sorted(excluded))
    if function == "provider_chat_names":
        provider_chat = ModelParamHelper.get_litellm_provider_specific_params_for_chat_params()
        return cast(Json, sorted(provider_chat))
    if function == "reasoning_effort":
        return _reasoning_effort(step)
    if function == "resolved_service_tier":
        return get_service_tier_from_standard_logging_payload(_service_tier_payload(step))
    raise ValueError(f"unknown parameter_projections function: {function}")


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    return [_run_step(_object(step, "step")) for step in steps]
