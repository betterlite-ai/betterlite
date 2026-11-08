import json
from collections.abc import Callable
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
    PromptTokensDetailsWrapper,
    StreamingChoices,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Step = dict[str, Json]

DELTA_FIELDS = frozenset(
    {
        "content",
        "role",
        "function_call",
        "tool_calls",
        "audio",
        "images",
        "annotations",
        "reasoning_content",
        "thinking_blocks",
        "provider_specific_fields",
    }
)
CHOICE_FIELDS = frozenset({"index", "delta", "finish_reason", "logprobs"})
CHUNK_FIELDS = frozenset(
    {
        "id",
        "object",
        "created",
        "model",
        "system_fingerprint",
        "choices",
        "usage",
        "provider_specific_fields",
    }
)
USAGE_FIELDS = frozenset(
    {
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "cost",
        "prompt_tokens_details",
        "completion_tokens_details",
    }
)


class _Logging:
    optional_params: dict[str, Json]
    model_call_details: dict[str, Json]
    messages: None

    def __init__(self) -> None:
        self.model_call_details = {"litellm_params": {}, "custom_llm_provider": None}
        self.optional_params = {}
        self.messages = None


def _object(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError("an object is required")
    return value


def _objects(value: Json) -> list[dict[str, Json]]:
    if not isinstance(value, list):
        raise ValueError("a list is required")
    return [_object(item) for item in value]


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _optional_string(value: Json) -> str | None:
    if value is None:
        return None
    return _string(value, "value")


def _strings(value: Json) -> list[str]:
    if not isinstance(value, list):
        raise ValueError("a list is required")
    return [_string(item, "item") for item in value]


def _raw(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast(Json, value.model_dump())
    return cast(Json, json.loads(json.dumps(value)))


def _non_null_object(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast(Json, _details_value(value))
    return _raw(value)


def _details_value(value: BaseModel) -> dict[str, Json]:
    dumped: dict[str, Json] = {}
    for key, item in cast(dict[str, Json], value.model_dump()).items():
        if item is not None:
            dumped[key] = _non_null_object(item)
    return dumped


def _delta(delta: Delta) -> dict[str, Json]:
    dumped: dict[str, Json] = {}
    for key in DELTA_FIELDS:
        item: object = getattr(delta, key, None)
        if item is not None:
            dumped[key] = _non_null_object(item)
    for key, item in cast(dict[str, Json], delta.model_dump()).items():
        if key not in DELTA_FIELDS and item is not None:
            dumped[key] = _raw(item)
    return dumped


def _choice(choice: StreamingChoices) -> dict[str, Json]:
    dumped: dict[str, Json] = {"index": choice.index, "delta": _delta(choice.delta)}
    finish_reason: object = getattr(choice, "finish_reason", None)
    if finish_reason is not None:
        dumped["finish_reason"] = _raw(finish_reason)
    logprobs: object = getattr(choice, "logprobs", None)
    if logprobs is not None:
        dumped["logprobs"] = _raw(logprobs)
    for key, item in cast(dict[str, Json], choice.model_dump()).items():
        if key not in CHOICE_FIELDS and item is not None:
            dumped[key] = _raw(item)
    return dumped


def _usage(usage: Usage) -> dict[str, Json]:
    dumped: dict[str, Json] = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }
    cost: object = getattr(usage, "cost", None)
    if cost is not None:
        dumped["cost"] = _raw(cost)
    prompt_details: object = getattr(usage, "prompt_tokens_details", None)
    if isinstance(prompt_details, BaseModel):
        dumped["prompt_tokens_details"] = _details_value(prompt_details)
    completion_details: object = getattr(usage, "completion_tokens_details", None)
    if isinstance(completion_details, BaseModel):
        dumped["completion_tokens_details"] = _details_value(completion_details)
    for key, item in cast(dict[str, Json], usage.model_dump()).items():
        if key not in USAGE_FIELDS and item is not None:
            dumped[key] = _raw(item)
    return dumped


def _chunk(value: ModelResponseStream) -> dict[str, Json]:
    hidden_params = cast(Json, value._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    dumped: dict[str, Json] = {
        "id": value.id,
        "object": value.object,
        "created": value.created,
        "model": value.model,
        "choices": [_choice(choice) for choice in value.choices],
        "hidden_params": _raw(hidden_params),
    }
    if value.system_fingerprint is not None:
        dumped["system_fingerprint"] = value.system_fingerprint
    usage: object = getattr(value, "usage", None)
    if isinstance(usage, Usage):
        dumped["usage"] = _usage(usage)
    if value.provider_specific_fields is not None:
        dumped["provider_specific_fields"] = cast(Json, value.provider_specific_fields)
    dumped_fields = cast(dict[str, Json], value.model_dump())  # pyright: ignore[reportUnknownMemberType]
    for key, item in dumped_fields.items():
        if key not in CHUNK_FIELDS:
            dumped[key] = _raw(item)
    return dumped


def _model_response(value: Json) -> ModelResponseStream:
    data = dict(_object(value))
    hidden_params = data.pop("hidden_params", {})
    response = ModelResponseStream(**data)  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]
    response._hidden_params = _object(hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    return response


def _response_object(value: Json) -> dict[str, Json]:
    data = _object(value)
    response: dict[str, Json] = {}
    original = data.get("original_chunk")
    if original is not None:
        response["original_chunk"] = cast(Json, _model_response(original))
    provider_fields = data.get("provider_specific_fields")
    if provider_fields is not None:
        response["provider_specific_fields"] = provider_fields
    return response


def _wrapper(state: Step) -> CustomStreamWrapper:
    logging = _Logging()
    provider = _optional_string(state.get("provider"))
    logging.model_call_details["custom_llm_provider"] = provider
    wrapper = CustomStreamWrapper(
        completion_stream=[],
        model=_string(state.get("model", ""), "model"),
        logging_obj=cast(LiteLLMLoggingObject, logging),
        custom_llm_provider=provider,
    )
    base_hidden = dict(_object(state.get("base_hidden_params", {})))
    wrapper._base_hidden_params = base_hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper.merge_reasoning_content_in_choices = (
        state.get("merge_reasoning_in_choices", False) is True
    )
    wrapper.response_id = _optional_string(state.get("response_id"))
    created = state.get("created")
    wrapper.created = None if created is None else _integer(created, "created")
    wrapper.system_fingerprint = _optional_string(state.get("system_fingerprint"))
    wrapper.sent_first_chunk = state.get("sent_first_chunk", False) is True
    wrapper.sent_last_chunk = state.get("sent_last_chunk", False) is True
    wrapper.sent_first_thinking_block = (
        state.get("sent_first_thinking_block", False) is True
    )
    wrapper.sent_last_thinking_block = (
        state.get("sent_last_thinking_block", False) is True
    )
    wrapper.holding_chunk = _string(state.get("holding_chunk", ""), "holding_chunk")
    wrapper.response_uptil_now = _string(
        state.get("response_uptil_now", ""), "response_uptil_now"
    )
    wrapper.received_finish_reason = _optional_string(
        state.get("received_finish_reason")
    )
    wrapper.intermittent_finish_reason = _optional_string(
        state.get("intermittent_finish_reason")
    )
    wrapper.tool_call = state.get("tool_call", False) is True
    wrapper.chunks = [  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
        _model_response(item) for item in _objects(state.get("chunks", []))
    ]
    count = state.get("repeated_messages_count", 1)
    repeated = _integer(count, "repeated_messages_count")
    wrapper._repeated_messages_count = repeated  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    return wrapper


def _snapshot(wrapper: CustomStreamWrapper) -> dict[str, Json]:
    return {
        "response_id": wrapper.response_id,
        "created": wrapper.created,
        "system_fingerprint": wrapper.system_fingerprint,
        "sent_first_chunk": wrapper.sent_first_chunk,
        "sent_last_chunk": wrapper.sent_last_chunk,
        "sent_first_thinking_block": wrapper.sent_first_thinking_block,
        "sent_last_thinking_block": wrapper.sent_last_thinking_block,
        "holding_chunk": wrapper.holding_chunk,
        "response_uptil_now": wrapper.response_uptil_now,
        "received_finish_reason": wrapper.received_finish_reason,
        "intermittent_finish_reason": wrapper.intermittent_finish_reason,
        "tool_call": wrapper.tool_call,
        "repeated_messages_count": wrapper._repeated_messages_count,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "chunk_count": len(cast(list[object], wrapper.chunks)),
    }


def _creator(wrapper: CustomStreamWrapper, step: Step) -> Json:
    chunk = step.get("chunk")
    hidden = step.get("hidden_params")
    created = wrapper.model_response_creator(  # pyright: ignore[reportUnknownMemberType]
        chunk=None if chunk is None else dict(_object(chunk)),
        hidden_params=None if hidden is None else _object(hidden),
    )
    return _chunk(created)


def _set_model_id(wrapper: CustomStreamWrapper, step: Step) -> Json:
    model_response = _model_response(step["chunk"])
    return _chunk(wrapper.set_model_id(_string(step["id"], "id"), model_response))


def _copy_provider_fields(wrapper: CustomStreamWrapper, step: Step) -> Json:
    model_response = _model_response(step["chunk"])
    original = _model_response(step["original"])
    copied = wrapper.copy_model_response_level_provider_specific_fields(
        original, model_response
    )
    return _chunk(copied)


def _is_chunk_non_empty(wrapper: CustomStreamWrapper, step: Step) -> Json:
    completion = dict(_object(step["completion"]))
    model_response = _model_response(step["chunk"])
    response = _response_object(step.get("response", {}))
    return wrapper.is_chunk_non_empty(completion, model_response, response)


def _returns(wrapper: CustomStreamWrapper, step: Step) -> Json:
    completion = dict(_object(step["completion"]))
    model_response = _model_response(step["chunk"])
    response = _response_object(step.get("response", {}))
    returned = wrapper.return_processed_chunk_logic(
        completion, model_response, response
    )
    return None if returned is None else _chunk(returned)


def _combine_thinking(wrapper: CustomStreamWrapper, step: Step) -> Json:
    model_response = _model_response(step["chunk"])
    wrapper._optional_combine_thinking_block_in_choices(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        model_response=model_response
    )
    return _chunk(model_response)


def _special_tokens(wrapper: CustomStreamWrapper, step: Step) -> Json:
    hold, text = wrapper.check_special_tokens(  # pyright: ignore[reportUnknownMemberType]
        _string(step["chunk"], "chunk"), _optional_string(step.get("finish_reason"))
    )
    return {"hold": hold, "text": text}


def _repetition(wrapper: CustomStreamWrapper, _step: Step) -> Json:
    wrapper.raise_on_model_repetition()
    return None


def _finish_reason(wrapper: CustomStreamWrapper, _step: Step) -> Json:
    return _chunk(wrapper.finish_reason_handler())


def _record_usage(wrapper: CustomStreamWrapper, step: Step) -> Json:
    wrapper._record_usage_only_chunk(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        model_response=_model_response(step["chunk"])
    )
    return None


def _strip_sse(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    line = step.get("chunk")
    return CustomStreamWrapper._strip_sse_data_from_chunk(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        None if line is None else _string(line, "chunk")
    )


def _generic_known(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return shaping.generic_chunk_has_all_required_fields(  # pyright: ignore[reportUnknownMemberType]
        dict(_object(step["chunk"]))
    )


def _convert_generic(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    chunk = dict(_object(step["chunk"]))
    converted = shaping.convert_generic_chunk_to_model_response_stream(
        chunk  # pyright: ignore[reportArgumentType]
    )
    return _chunk(converted)


def _total_usage(wrapper: CustomStreamWrapper, _step: Step) -> Json:
    return _usage(
        shaping.calculate_total_usage(  # pyright: ignore[reportUnknownMemberType]
            cast(list[ModelResponse], wrapper.chunks)
        )
    )


def _backfill_usage(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    usage = Usage(
        **dict(_object(step["usage"]))  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]
    )
    shaping.backfill_missing_cache_usage_fields(usage)
    return _usage(usage)


def _resolve_cost(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return CustomStreamWrapper._resolve_provider_reported_cost(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        step["cost"]
    )


def _propagate_cost(wrapper: CustomStreamWrapper, step: Step) -> Json:
    model_response = _model_response(step["chunk"])
    wrapper._propagate_usage_cost_to_hidden_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(ModelResponse, model_response), wrapper.custom_llm_provider
    )
    return _chunk(model_response)


def _cache_token_count(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    details = step.get("details")
    counted = (
        None
        if details is None
        else PromptTokensDetailsWrapper(  # pyright: ignore[reportUnknownArgumentType]
            **dict(_object(details))
        )
    )
    keys = tuple(_strings(step["keys"]))
    count: Json = shaping._cache_token_count(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        counted, keys
    )
    return count


def _coerce_details(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    wrapped = PromptTokensDetailsWrapper(  # pyright: ignore[reportUnknownArgumentType]
        **dict(_object(step["details"]))
    )
    coerced = shaping._coerce_token_details(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        {"prompt_tokens_details": wrapped},
        "prompt_tokens_details",
        PromptTokensDetailsWrapper,
    )
    return None if coerced is None else _details_value(coerced)


def _provider_response_model(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return shaping._provider_response_model(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        _model_response(step["chunk"])
    )


def _provider_hidden_params(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    params = shaping._provider_hidden_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        _model_response(step["chunk"]),
        _optional_string(step.get("provider_response_model")),
    )
    return None if params is None else cast(Json, dict(params))


def _stamp_service_tier(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    model_response = _model_response(step["chunk"])
    complete = _model_response(step["complete"])
    shaping._stamp_served_service_tier(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        model_response, cast(ModelResponse, complete)
    )
    return _chunk(model_response)


def _check_stream_usage(wrapper: CustomStreamWrapper, step: Step) -> Json:
    options = step.get("stream_options")
    return wrapper.check_send_stream_usage(  # pyright: ignore[reportUnknownMemberType]
        None if options is None else dict(_object(options))
    )


def _check_function_call(wrapper: CustomStreamWrapper, step: Step) -> Json:
    logging = _Logging()
    logging.optional_params = dict(_object(step["optional_params"]))
    return wrapper.check_is_function_call(  # pyright: ignore[reportUnknownMemberType]
        cast(LiteLLMLoggingObject, logging)
    )


def _set_state(wrapper: CustomStreamWrapper, step: Step) -> Json:
    patch = _object(step["patch"])
    if patch.get("sent_first_chunk") is not None:
        wrapper.sent_first_chunk = patch["sent_first_chunk"] is True
    if patch.get("sent_last_chunk") is not None:
        wrapper.sent_last_chunk = patch["sent_last_chunk"] is True
    if patch.get("sent_first_thinking_block") is not None:
        wrapper.sent_first_thinking_block = patch["sent_first_thinking_block"] is True
    if patch.get("sent_last_thinking_block") is not None:
        wrapper.sent_last_thinking_block = patch["sent_last_thinking_block"] is True
    if patch.get("holding_chunk") is not None:
        wrapper.holding_chunk = _string(patch["holding_chunk"], "holding_chunk")
    if patch.get("received_finish_reason") is not None:
        wrapper.received_finish_reason = _optional_string(patch["received_finish_reason"])
    if patch.get("intermittent_finish_reason") is not None:
        wrapper.intermittent_finish_reason = _optional_string(
            patch["intermittent_finish_reason"]
        )
    if patch.get("tool_call") is not None:
        wrapper.tool_call = patch["tool_call"] is True
    return None


def _mapped_finish_reason(_wrapper: CustomStreamWrapper, step: Step) -> Json:
    return shaping.map_finish_reason(_string(step["raw"], "raw"))


_HANDLERS: dict[str, Callable[[CustomStreamWrapper, Step], Json]] = {
    "creator": _creator,
    "set_model_id": _set_model_id,
    "copy_provider_fields": _copy_provider_fields,
    "is_chunk_non_empty": _is_chunk_non_empty,
    "returns": _returns,
    "combine_thinking": _combine_thinking,
    "special_tokens": _special_tokens,
    "repetition": _repetition,
    "finish_reason": _finish_reason,
    "record_usage": _record_usage,
    "strip_sse": _strip_sse,
    "generic_known": _generic_known,
    "convert_generic": _convert_generic,
    "total_usage": _total_usage,
    "backfill_usage": _backfill_usage,
    "resolve_cost": _resolve_cost,
    "propagate_cost": _propagate_cost,
    "cache_token_count": _cache_token_count,
    "coerce_details": _coerce_details,
    "provider_response_model": _provider_response_model,
    "provider_hidden_params": _provider_hidden_params,
    "stamp_service_tier": _stamp_service_tier,
    "check_stream_usage": _check_stream_usage,
    "check_function_call": _check_function_call,
    "mapped_finish_reason": _mapped_finish_reason,
    "set_state": _set_state,
}


def _run_step(wrapper: CustomStreamWrapper, step: Step) -> Json:
    op = _string(step.get("op", ""), "op")
    handler = _HANDLERS.get(op)
    if handler is None:
        raise ValueError(f"unsupported operation {op}")
    return handler(wrapper, step)


def run(input_value: dict[str, Json]) -> Json:
    state = _object(input_value.get("state", {}))
    limit = _integer(input_value["repeated_chunk_limit"], "repeated_chunk_limit")
    litellm.REPEATED_STREAMING_CHUNK_LIMIT = limit  # pyright: ignore[reportPrivateImportUsage]
    providers = frozenset(_strings(input_value.get("usage_cost_providers", [])))
    shaping._USAGE_COST_HEADER_PROVIDERS = providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper = _wrapper(state)
    outputs: list[Json] = [
        {"result": _run_step(wrapper, step), "state": _snapshot(wrapper)}
        for step in _objects(input_value["steps"])
    ]
    return cast(Json, outputs)
