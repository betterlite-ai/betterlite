import json
from collections.abc import Callable
from types import SimpleNamespace
from typing import TypeAlias, cast

import litellm
from litellm.litellm_core_utils import streaming_handler as shaping
from litellm.litellm_core_utils.redact_messages import LiteLLMLoggingObject
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.types.utils import (
    BaseModel,
    Delta,
    ModelResponse,
    ModelResponseStream,
    StreamingChoices,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Step: TypeAlias = dict[str, Json]

DELTA_FIELDS = [
    "content", "role", "function_call", "tool_calls", "audio", "images", "annotations",
    "reasoning_content", "thinking_blocks", "provider_specific_fields",
]
DETAIL_FIELDS = [
    "cached_tokens", "cache_write_tokens", "cache_creation_tokens", "cache_creation_token_details",
]


def _as_object(value: Json) -> dict[str, Json]:
    return cast("dict[str, Json]", value)


def _as_text(value: Json) -> str:
    return cast(str, value)


def _as_int(value: Json) -> int:
    return cast(int, value)


def _as_list(value: Json) -> list[Json]:
    return cast("list[Json]", value)


def _attr(value: object, key: str) -> object:
    return getattr(value, key, None)


def _optional(state: Step, key: str) -> str | None:
    value = state.get(key)
    return None if value is None else _as_text(value)


def _optional_int(state: Step, key: str) -> int | None:
    value = state.get(key)
    return None if value is None else _as_int(value)


def _flag(state: Step, key: str) -> bool:
    return state.get(key) is True


def _raw(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast("Json", value.model_dump())
    return cast("Json", json.loads(json.dumps(value)))


def _dump(value: BaseModel) -> dict[str, Json]:
    return cast("dict[str, Json]", value.model_dump())


def _delta(delta: Delta) -> dict[str, Json]:
    dumped: dict[str, Json] = {}
    for key in DELTA_FIELDS:
        item: object = getattr(delta, key, None)
        dumped[key] = _raw(item)
    for key, item in _dump(delta).items():
        if key not in DELTA_FIELDS:
            dumped[key] = _raw(item)
    return dumped


def _choice(choice: StreamingChoices) -> dict[str, Json]:
    dumped = _dump(choice)
    dumped["delta"] = _delta(choice.delta)
    return dumped


def _details(details: BaseModel) -> dict[str, Json]:
    return {key: _raw(item) for key, item in _dump(details).items() if item is not None}


def _usage(usage: Usage) -> dict[str, Json]:
    prompt = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    dumped = _dump(usage)
    dumped["prompt_tokens_details"] = None if prompt is None else _details(prompt)
    dumped["completion_tokens_details"] = None if completion is None else _details(completion)
    return dumped


def _chunk(value: ModelResponseStream) -> dict[str, Json]:
    usage: object = getattr(value, "usage", None)
    if isinstance(usage, dict):
        usage = Usage(**cast("dict[str, object]", usage))  # pyright: ignore[reportArgumentType]
    dumped = _dump(value)
    dumped["choices"] = [_choice(choice) for choice in value.choices]
    if isinstance(usage, Usage):
        dumped["usage"] = _usage(usage)
    dumped["hidden_params"] = _raw(_attr(value, "_hidden_params"))
    return dumped


def _model_response(value: Json) -> ModelResponseStream:
    data = dict(_as_object(value))
    hidden = data.pop("hidden_params", {})
    response = ModelResponseStream(**data)  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]
    response._hidden_params = _as_object(hidden)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    return response


def _response_object(value: Json) -> dict[str, Json]:
    data = _as_object(value)
    original = data.get("original_chunk")
    fields = data.get("provider_specific_fields")
    response: dict[str, Json] = {}
    if original is not None:
        response["original_chunk"] = cast("Json", _model_response(original))
    if fields is not None:
        response["provider_specific_fields"] = fields
    return response


def _wrapper(state: Step) -> CustomStreamWrapper:
    provider = _optional(state, "provider")
    logging = SimpleNamespace(optional_params={}, messages=None)
    logging.model_call_details = {"litellm_params": {}, "custom_llm_provider": provider}
    wrapper = CustomStreamWrapper(
        completion_stream=[],
        model=_as_text(state.get("model", "")),
        logging_obj=cast(LiteLLMLoggingObject, logging),
        custom_llm_provider=provider,
    )
    wrapper._base_hidden_params = dict(_as_object(state.get("base_hidden_params", {})))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper.merge_reasoning_content_in_choices = _flag(state, "merge_reasoning_in_choices")
    wrapper.response_id = _optional(state, "response_id")
    wrapper.created = _optional_int(state, "created")
    wrapper.system_fingerprint = _optional(state, "system_fingerprint")
    wrapper.tool_call = _flag(state, "tool_call")
    wrapper.chunks = [_model_response(item) for item in _as_list(state.get("chunks", []))]
    return wrapper


def _creator(wrapper: CustomStreamWrapper, step: Step) -> Json:
    chunk = step.get("chunk")
    hidden = step.get("hidden_params")
    return _chunk(
        wrapper.model_response_creator(  # pyright: ignore[reportUnknownMemberType]
            chunk=None if chunk is None else _as_object(chunk),
            hidden_params=None if hidden is None else _as_object(hidden),
        )
    )


def _set_model_id(wrapper: CustomStreamWrapper, step: Step) -> Json:
    response = _model_response(step.get("chunk", {}))
    return _chunk(wrapper.set_model_id(_as_text(step.get("id", "")), response))


def _is_chunk_non_empty(wrapper: CustomStreamWrapper, step: Step) -> Json:
    return wrapper.is_chunk_non_empty(
        dict(_as_object(step.get("completion", {}))),
        _model_response(step.get("chunk", {})),
        _response_object(step.get("response", {})),
    )


def _returns(wrapper: CustomStreamWrapper, step: Step) -> Json:
    returned = wrapper.return_processed_chunk_logic(
        dict(_as_object(step.get("completion", {}))),
        _model_response(step.get("chunk", {})),
        _response_object(step.get("response", {})),
    )
    return None if returned is None else _chunk(returned)


def _combine_thinking(wrapper: CustomStreamWrapper, step: Step) -> Json:
    response = _model_response(step.get("chunk", {}))
    wrapper._optional_combine_thinking_block_in_choices(model_response=response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return _chunk(response)


def _special_tokens(wrapper: CustomStreamWrapper, step: Step) -> Json:
    hold, text = wrapper.check_special_tokens(
        _as_text(step.get("chunk", "")), _optional(step, "finish_reason")
    )
    return {"hold": hold, "text": text}


def _repetition(wrapper: CustomStreamWrapper, _step: Step) -> Json:
    wrapper.raise_on_model_repetition()
    return None


def _finish_reason(wrapper: CustomStreamWrapper, _step: Step) -> Json:
    return _chunk(wrapper.finish_reason_handler())


def _strip_sse(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return CustomStreamWrapper._strip_sse_data_from_chunk(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _optional(step, "chunk")
    )


def _generic_known(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return shaping.generic_chunk_has_all_required_fields(  # pyright: ignore[reportUnknownMemberType]
        dict(_as_object(step.get("chunk", {})))
    )


def _convert_generic(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return _chunk(
        shaping.convert_generic_chunk_to_model_response_stream(
            dict(_as_object(step.get("chunk", {})))  # pyright: ignore[reportArgumentType]
        )
    )


def _total_usage(wrapper: CustomStreamWrapper, _step: Step) -> Json:
    return _usage(shaping.calculate_total_usage(cast("list[ModelResponse]", wrapper.chunks)))


def _backfill_usage(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    usage = Usage(**cast("dict[str, object]", _as_object(step.get("usage", {}))))  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]
    shaping.backfill_missing_cache_usage_fields(usage)
    return _usage(usage)


def _propagate_cost(wrapper: CustomStreamWrapper, step: Step) -> Json:
    response = _model_response(step.get("chunk", {}))
    wrapper._propagate_usage_cost_to_hidden_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(ModelResponse, response), wrapper.custom_llm_provider
    )
    return _chunk(response)


def _provider_response_model(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return shaping._provider_response_model(_model_response(step.get("chunk", {})))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _provider_hidden_params(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    params = shaping._provider_hidden_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _model_response(step.get("chunk", {})), _optional(step, "provider_response_model")
    )
    return None if params is None else cast("Json", dict(params))


def _stamp_service_tier(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    response = _model_response(step.get("chunk", {}))
    complete = _model_response(step.get("complete", {}))
    shaping._stamp_served_service_tier(response, cast(ModelResponse, complete))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return _chunk(response)


def _set_state(wrapper: CustomStreamWrapper, step: Step) -> Json:
    for key, value in _as_object(step.get("patch", {})).items():
        setattr(wrapper, key, value)
    return None


_HANDLERS: dict[str, Callable[[CustomStreamWrapper, Step], Json]] = {
    "creator": _creator,
    "set_model_id": _set_model_id,
    "is_chunk_non_empty": _is_chunk_non_empty,
    "returns": _returns,
    "combine_thinking": _combine_thinking,
    "special_tokens": _special_tokens,
    "repetition": _repetition,
    "finish_reason": _finish_reason,
    "strip_sse": _strip_sse,
    "generic_known": _generic_known,
    "convert_generic": _convert_generic,
    "total_usage": _total_usage,
    "backfill_usage": _backfill_usage,
    "propagate_cost": _propagate_cost,
    "provider_response_model": _provider_response_model,
    "provider_hidden_params": _provider_hidden_params,
    "stamp_service_tier": _stamp_service_tier,
    "set_state": _set_state,
}


def run(input_value: dict[str, Json]) -> Json:
    limit = _as_int(input_value.get("repeated_chunk_limit", 0))
    litellm.REPEATED_STREAMING_CHUNK_LIMIT = limit  # pyright: ignore[reportPrivateImportUsage]
    listed = _as_list(input_value.get("usage_cost_providers", []))
    providers = frozenset(_as_text(item) for item in listed)
    shaping._USAGE_COST_HEADER_PROVIDERS = providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper = _wrapper(_as_object(input_value.get("state", {})))
    steps = _as_list(input_value.get("steps", []))
    return cast("Json", [{"result": _run_step(wrapper, _as_object(step))} for step in steps])


def _run_step(wrapper: CustomStreamWrapper, step: Step) -> Json:
    op = _as_text(step.get("op", ""))
    handler = _HANDLERS.get(op)
    if handler is None:
        raise ValueError(f"unsupported operation {op}")
    return handler(wrapper, step)
