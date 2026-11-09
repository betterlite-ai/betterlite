from collections.abc import Callable
from typing import TypeAlias, cast

import httpx
from litellm.cost_calculator import (  # pyright: ignore[reportPrivateUsage]
    _infer_call_type,  # pyright: ignore[reportPrivateUsage]
    _is_known_usage_objects,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _resolve_billable_service_tier,  # pyright: ignore[reportPrivateUsage]
    _served_service_tier,  # pyright: ignore[reportPrivateUsage]
    _transcription_usage_has_token_details,  # pyright: ignore[reportPrivateUsage]
    _without_provider_stated_cost,  # pyright: ignore[reportPrivateUsage]
    get_response_cost_from_hidden_params,  # pyright: ignore[reportUnknownVariableType]
    get_usage_object,
)
from litellm.types.agents import LiteLLMSendMessageResponse
from litellm.types.decisions import (
    DecisionsResponse,
    DecisionsUsage,
    OpenAIDecisionResponse,
    OpenAIDecisionUsage,
)
from litellm.types.llms.openai import HttpxBinaryResponseContent, ResponseAPIUsage
from litellm.types.rerank import RerankResponse
from litellm.types.utils import (
    EmbeddingResponse,
    ImageResponse,
    ModelResponse,
    ModelResponseStream,
    TextCompletionResponse,
    TranscriptionResponse,
    TranscriptionUsageDurationObject,
    TranscriptionUsageTokensObject,
    Usage,
)
from pydantic import BaseModel, ConfigDict, PrivateAttr

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
_PROMPT_FIELDS = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "cached_tokens",
    "cache_write_tokens",
)
_COMPLETION_FIELDS = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "reasoning_tokens",
)
_RESPONSE_FACTORIES: dict[str, Callable[[], object]] = {
    "chat": ModelResponse,
    "chat_stream": ModelResponseStream,
    "embedding": EmbeddingResponse,
    "transcription": TranscriptionResponse,
    "rerank": RerankResponse,
    "image": ImageResponse,
    "text_completion": TextCompletionResponse,
    "send_message": LiteLLMSendMessageResponse.model_construct,
}


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


def _details_projection(value: object, fields: tuple[str, ...]) -> dict[str, Json] | None:
    if value is None:
        return None
    return {
        field: cast(Json, found)
        for field in fields
        if (found := getattr(value, field, None)) is not None
    }


def _usage_projection(usage: Usage | None) -> Json:
    if usage is None:
        return None
    result: dict[str, Json] = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }
    prompt = _details_projection(usage.prompt_tokens_details, _PROMPT_FIELDS)
    if prompt is not None:
        result["prompt_tokens_details"] = prompt
    completion = _details_projection(usage.completion_tokens_details, _COMPLETION_FIELDS)
    if completion is not None:
        result["completion_tokens_details"] = completion
    cost: object = getattr(usage, "cost", None)
    if cost is not None:
        result["cost"] = cast(Json, cost)
    result["extras"] = cast(Json, usage.model_extra or {})
    return result


def _transcription_usage(value: dict[str, Json]) -> object:
    if value.get("type") == "duration":
        return TranscriptionUsageDurationObject.model_validate(value)
    return TranscriptionUsageTokensObject.model_validate(value)


def _chat_usage(mapping: dict[str, Json]) -> Usage:
    return cast("Callable[..., Usage]", Usage)(**mapping)


def _decisions_usage(mapping: dict[str, Json]) -> DecisionsUsage:
    return cast("Callable[..., DecisionsUsage]", DecisionsUsage)(**mapping)


def _openai_decisions_usage(mapping: dict[str, Json]) -> OpenAIDecisionUsage:
    return cast("Callable[..., OpenAIDecisionUsage]", OpenAIDecisionUsage)(**mapping)


def _provider_usage_object(kind: str, value: Json) -> object:
    match kind:
        case "chat":
            return _chat_usage(_object(value, "value"))
        case "responses":
            return ResponseAPIUsage.model_validate(value)
        case "transcription":
            return _transcription_usage(_object(value, "value"))
        case "decisions_ir":
            return _decisions_usage(_object(value, "value"))
        case "decisions_openai":
            return _openai_decisions_usage(_object(value, "value"))
        case _:
            raise ValueError(f"unknown provider usage kind {kind}")


def _usage_object(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    if kind in ("absent", "null"):
        return None
    if kind == "typed_record":
        return _TypedUsage(**_object(spec.get("value"), "value"))
    if kind in ("interactions", "mapping"):
        return _object(spec.get("value"), "value")
    return _provider_usage_object(kind, spec.get("value"))


def _decisions_ir_response(spec: dict[str, Json] | None) -> DecisionsResponse:
    if spec is None:
        return DecisionsResponse(model=None, answers={}, usage=None)
    usage = cast("DecisionsUsage | None", _usage_object(spec))
    return DecisionsResponse(model=None, answers={}, usage=usage)


def _decisions_openai_response(spec: dict[str, Json]) -> OpenAIDecisionResponse:
    usage = cast("OpenAIDecisionUsage", _usage_object(spec))
    return OpenAIDecisionResponse(model="model", answers=(), usage=usage)


def _response_object(spec: dict[str, Json]) -> object:
    kind = _string(spec.get("kind"), "kind")
    raw_usage = spec.get("usage")
    usage_spec = None if raw_usage is None else _object(raw_usage, "usage")
    if kind == "decisions_ir":
        return _decisions_ir_response(usage_spec)
    if kind == "decisions_openai":
        if usage_spec is None:
            raise ValueError("an OpenAI Decisions response requires a usage object")
        return _decisions_openai_response(usage_spec)
    usage = None if usage_spec is None else _usage_object(usage_spec)
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
    return _chat_usage(_object(raw_usage, "usage"))


def _hidden_params(call: dict[str, Json]) -> dict[str, Json]:
    raw_hidden = call.get("hidden_params")
    return {} if raw_hidden is None else _object(raw_hidden, "hidden_params")


def _infer_response(kind: str) -> object:
    if kind == "speech":
        return HttpxBinaryResponseContent(httpx.Response(200))
    factory = _RESPONSE_FACTORIES.get(kind)
    if factory is None:
        return {}
    return factory()


def _execute(function: str, call: dict[str, Json]) -> Json:
    match function:
        case "get_usage_object":
            response = _response_object(_object(call.get("response"), "response"))
            result: Json = _usage_projection(get_usage_object(response))
        case "is_known_usage":
            variant = _usage_object(_object(call.get("variant"), "variant"))
            result = _is_known_usage_objects(variant)
        case "infer_call_type":
            infer = cast("Callable[[str | None, object], str | None]", _infer_call_type)
            inferred = infer(
                _optional_string(call.get("call_type"), "call_type"),
                _infer_response(_string(call.get("kind"), "kind")),
            )
            result = cast(Json, inferred)
        case "transcription_has_token_details":
            result = _transcription_usage_has_token_details(_optional_usage(call))
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
    function = _string(call.get("function"), "function")
    try:
        value = _execute(function, call)
    except Exception as error:  # noqa: BLE001 - a rejected probe is part of the compared outcome
        outcome: dict[str, Json] = {"ok": False, "error": {"class": type(error).__name__}}
        return outcome
    return {"ok": True, "value": value}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_run_call(_object(call, "call")) for call in calls]
