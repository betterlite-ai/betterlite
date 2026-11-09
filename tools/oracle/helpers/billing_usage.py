from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.cost_calculator import (  # pyright: ignore[reportPrivateUsage]
    _resolve_billable_service_tier,  # pyright: ignore[reportPrivateUsage]
    _served_service_tier,  # pyright: ignore[reportPrivateUsage]
    _without_provider_stated_cost,  # pyright: ignore[reportPrivateUsage]
    get_response_cost_from_hidden_params,  # pyright: ignore[reportUnknownVariableType]
    get_usage_object,
)
from litellm.types.decisions import (
    DecisionsResponse,
    DecisionsUsage,
    OpenAIDecisionResponse,
    OpenAIDecisionUsage,
)
from litellm.types.llms.openai import ResponseAPIUsage
from litellm.types.utils import (
    TranscriptionUsageDurationObject,
    TranscriptionUsageTokensObject,
    Usage,
)
from pydantic import BaseModel, ConfigDict, PrivateAttr

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
_PROMPT_FIELDS = ("text_tokens", "audio_tokens", "image_tokens", "video_tokens",
                  "cached_tokens", "cache_write_tokens")
_COMPLETION_FIELDS = ("text_tokens", "audio_tokens", "image_tokens", "video_tokens",
                      "reasoning_tokens")


class _TypedUsage(BaseModel):
    model_config = ConfigDict(extra="allow")


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
    projected: dict[str, Json] = {}
    for field in fields:
        found = getattr(value, field, None)
        if found is not None:
            projected[field] = cast(Json, found)
    return projected


def _usage_projection(usage: Usage | None) -> Json:
    if usage is None:
        return None
    result: dict[str, Json] = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }
    prompt = _details(usage.prompt_tokens_details, _PROMPT_FIELDS)
    if prompt is not None:
        result["prompt_tokens_details"] = prompt
    completion = _details(usage.completion_tokens_details, _COMPLETION_FIELDS)
    if completion is not None:
        result["completion_tokens_details"] = completion
    cost: object = getattr(usage, "cost", None)
    if cost is not None:
        result["cost"] = cast(Json, cost)
    result["extras"] = cast(Json, usage.model_extra or {})
    return result


def _usage_object(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    value = spec.get("value")
    if kind in ("absent", "null"):
        return None
    if kind == "typed_record":
        return _TypedUsage(**_object(value, "value"))
    if kind in ("interactions", "mapping"):
        return _object(value, "value")
    return _provider_usage_object(kind, value)


def _provider_usage_object(kind: str, value: Json) -> object:
    if kind == "chat":
        return cast("Callable[..., Usage]", Usage)(**_object(value, "value"))
    if kind == "responses":
        return ResponseAPIUsage.model_validate(value)
    if kind == "transcription":
        inner = _object(value, "value")
        if inner.get("type") == "duration":
            return TranscriptionUsageDurationObject.model_validate(inner)
        return TranscriptionUsageTokensObject.model_validate(inner)
    if kind == "decisions_ir":
        usage_input = cast("Callable[..., DecisionsUsage]", DecisionsUsage)
        return usage_input(**_object(value, "value"))
    if kind == "decisions_openai":
        openai_input = cast("Callable[..., OpenAIDecisionUsage]", OpenAIDecisionUsage)
        return openai_input(**_object(value, "value"))
    raise ValueError(f"unknown provider usage kind {kind}")


def _response_object(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    raw_usage = spec.get("usage")
    usage_spec = None if raw_usage is None else _object(raw_usage, "usage")
    usage = None if usage_spec is None else _usage_object(usage_spec)
    if kind == "decisions_ir":
        return DecisionsResponse(model=None, answers={}, usage=cast("DecisionsUsage | None", usage))
    if kind == "decisions_openai":
        decisions_usage = cast("OpenAIDecisionUsage", usage)
        return OpenAIDecisionResponse(model="model", answers=(), usage=decisions_usage)
    response = _Response(usage=usage)
    raw_tier = spec.get("service_tier")
    if raw_tier is not None:
        response.service_tier = _string(raw_tier, "service_tier")  # pyright: ignore[reportAttributeAccessIssue]
    raw_hidden = spec.get("hidden_params")
    if raw_hidden is not None:
        response._hidden_params = _object(raw_hidden, "hidden_params")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return response


def _optional_usage(call: dict[str, Json]) -> Usage | None:
    raw_usage = call.get("usage")
    if raw_usage is None:
        return None
    return cast("Callable[..., Usage]", Usage)(**_object(raw_usage, "usage"))


def _hidden_params(call: dict[str, Json]) -> dict[str, Json]:
    raw_hidden = call.get("hidden_params")
    return {} if raw_hidden is None else _object(raw_hidden, "hidden_params")


def _execute(function: str, call: dict[str, Json]) -> Json:
    match function:
        case "get_usage_object":
            response = _response_object(_object(call.get("response"), "response"))
            result: Json = _usage_projection(get_usage_object(response))
        case "without_provider_stated_cost":
            result = _usage_projection(_without_provider_stated_cost(_optional_usage(call)))
        case "hidden_response_cost":
            result = cast(Json, get_response_cost_from_hidden_params(_hidden_params(call)))
        case "served_service_tier":
            response = _response_object(_object(call.get("response"), "response"))
            result = cast(Json, _served_service_tier(response, _optional_usage(call)))
        case "resolve_billable_service_tier":
            resolved = _resolve_billable_service_tier(
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
