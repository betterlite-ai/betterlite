from typing import TYPE_CHECKING, TypeAlias, cast

from litellm import cost_calculator
from litellm.types import decisions, utils
from litellm.types.llms import openai as openai_types
from pydantic import BaseModel, ConfigDict, PrivateAttr

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
_PROMPT_FIELDS = ("text_tokens", "audio_tokens", "image_tokens", "video_tokens",
                  "cached_tokens", "cache_write_tokens")
_COMPLETION_FIELDS = ("text_tokens", "audio_tokens", "image_tokens", "video_tokens",
                      "reasoning_tokens")


class _Response(BaseModel):
    model_config = ConfigDict(extra="allow")
    usage: object = None
    _hidden_params: dict[str, Json] = PrivateAttr(default_factory=dict)

    def get(self, key: str, default: object = None) -> object:
        return getattr(self, key, default)


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(value: Json | None, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _details(value: object, fields: tuple[str, ...]) -> dict[str, Json] | None:
    if value is None:
        return None
    present = (field for field in fields if getattr(value, field, None) is not None)
    return {field: cast(Json, getattr(value, field)) for field in present}


def _usage_projection(usage: utils.Usage | None) -> Json:
    if usage is None:
        return None
    result: dict[str, Json] = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }
    for name, details, fields in (
        ("prompt_tokens_details", usage.prompt_tokens_details, _PROMPT_FIELDS),
        ("completion_tokens_details", usage.completion_tokens_details, _COMPLETION_FIELDS),
    ):
        projected = _details(details, fields)
        if projected is not None:
            result[name] = projected
    if (cost := getattr(usage, "cost", None)) is not None:
        result["cost"] = cast(Json, cost)
    result["extras"] = cast(Json, usage.model_extra or {})
    return result


def _usage_object(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    value = spec.get("value")
    if kind in ("absent", "null"):
        return None
    if kind in ("interactions", "mapping"):
        return _object(value, "value")
    return _provider_usage_object(kind, value)


def _provider_usage_object(kind: str, value: Json) -> object:
    if kind == "chat":
        return cast("Callable[..., object]", utils.Usage)(**_object(value, "value"))
    if kind == "responses":
        return openai_types.ResponseAPIUsage.model_validate(value)
    if kind == "transcription":
        inner = _object(value, "value")
        if inner.get("type") == "duration":
            return utils.TranscriptionUsageDurationObject.model_validate(inner)
        return utils.TranscriptionUsageTokensObject.model_validate(inner)
    if kind == "decisions_ir":
        usage_input = cast("Callable[..., object]", decisions.DecisionsUsage)
        return usage_input(**_object(value, "value"))
    if kind == "decisions_openai":
        openai_input = cast("Callable[..., object]", decisions.OpenAIDecisionUsage)
        return openai_input(**_object(value, "value"))
    raise ValueError(f"unknown provider usage kind {kind}")


def _response_object(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    raw_usage = spec.get("usage")
    usage = None if raw_usage is None else _usage_object(_object(raw_usage, "usage"))
    if kind == "decisions_ir":
        typed = cast("decisions.DecisionsUsage | None", usage)
        return decisions.DecisionsResponse(model=None, answers={}, usage=typed)
    if kind == "decisions_openai":
        decisions_usage = cast("decisions.OpenAIDecisionUsage", usage)
        return decisions.OpenAIDecisionResponse(model="model", answers=(), usage=decisions_usage)
    response = _Response(usage=usage)
    if (tier := spec.get("service_tier")) is not None:
        response.service_tier = _string(tier, "service_tier")  # pyright: ignore[reportAttributeAccessIssue]
    if (hidden := spec.get("hidden_params")) is not None:
        response._hidden_params = _object(hidden, "hidden_params")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return response


def _optional_usage(call: dict[str, Json]) -> utils.Usage | None:
    raw_usage = call.get("usage")
    if raw_usage is None:
        return None
    builder = cast("Callable[..., utils.Usage]", utils.Usage)
    return builder(**_object(raw_usage, "usage"))


def _execute(function: str, call: dict[str, Json]) -> Json:
    match function:
        case "get_usage_object":
            response = _response_object(_object(call.get("response"), "response"))
            result: Json = _usage_projection(cost_calculator.get_usage_object(response))
        case "without_provider_stated_cost":
            cleared = cost_calculator._without_provider_stated_cost(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                _optional_usage(call)
            )
            result = _usage_projection(cleared)
        case "hidden_response_cost":
            raw_hidden = call.get("hidden_params")
            hidden = {} if raw_hidden is None else _object(raw_hidden, "hidden_params")
            cost = cost_calculator.get_response_cost_from_hidden_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                hidden
            )
            result = cast(Json, cost)
        case "served_service_tier":
            response = _response_object(_object(call.get("response"), "response"))
            served = cost_calculator._served_service_tier(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                response, _optional_usage(call)
            )
            result = cast(Json, served)
        case "resolve_billable_service_tier":
            resolved = cost_calculator._resolve_billable_service_tier(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                _optional_string(call.get("requested_tier"), "requested_tier"),
                _optional_string(call.get("served_tier"), "served_tier"),
            )
            result = cast(Json, resolved)
        case _:
            raise ValueError(f"unknown function {function}")
    return result


def _run_call(call: dict[str, Json]) -> Json:
    try:
        value = _execute(_string(call.get("function"), "function"), call)
    except Exception as error:  # noqa: BLE001 - a rejected probe is part of the compared outcome
        outcome: dict[str, Json] = {"ok": False, "error": {"class": type(error).__name__}}
        return outcome
    return {"ok": True, "value": value}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_run_call(_object(call, "call")) for call in calls]
