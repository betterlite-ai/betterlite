import json
from collections.abc import Callable
from types import SimpleNamespace
from typing import TypeAlias, cast

import litellm
from litellm.litellm_core_utils.redact_messages import LiteLLMLoggingObject
from litellm.litellm_core_utils.streaming_handler import (
    CustomStreamWrapper,
    _ProviderChunkEarlyReturn,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.utils import BaseModel, ModelResponseStream, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Step: TypeAlias = dict[str, Json]
Adapter: TypeAlias = Callable[[CustomStreamWrapper, Step, Step], Json]

OPENAI_READERS: frozenset[str | None] = frozenset(
    {"openai", "azure", "azure_ai", "cached_response"}
)


def _object(value: Json) -> dict[str, Json]:
    return cast("dict[str, Json]", value)


def _text(value: Json) -> str:
    return cast(str, value)


def _int(value: Json) -> int:
    return cast(int, value)


def _list(value: Json) -> list[Json]:
    return cast("list[Json]", value)


def _optional(state: Step, key: str) -> str | None:
    value = state.get(key)
    return None if value is None else _text(value)


def _optional_int(state: Step, key: str) -> int | None:
    value = state.get(key)
    return None if value is None else _int(value)


def _flag(state: Step, key: str) -> bool:
    return state.get(key) is True


def _attr(value: object, key: str) -> object:
    return getattr(value, key, None)


def _raw(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast("Json", value.model_dump())
    return cast("Json", json.loads(json.dumps(value)))


def _model_response(value: Json) -> ModelResponseStream:
    data = dict(_object(value))
    hidden = data.pop("hidden_params", None)
    response = ModelResponseStream(**data)  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]
    response._hidden_params = {} if hidden is None else dict(_object(hidden))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    return response


def _wrapper(state: Step) -> CustomStreamWrapper:
    provider = _optional(state, "provider")
    logging = SimpleNamespace(optional_params={}, messages=None)
    logging.model_call_details = {"litellm_params": {}, "custom_llm_provider": provider}
    wrapper = CustomStreamWrapper(
        completion_stream=[],
        model=_text(state.get("model", "")),
        logging_obj=cast(LiteLLMLoggingObject, logging),
        custom_llm_provider=provider,
    )
    wrapper._base_hidden_params = dict(_object(state.get("base_hidden_params", {})))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper.received_finish_reason = _optional(state, "received_finish_reason")
    wrapper.intermittent_finish_reason = _optional(state, "intermittent_finish_reason")
    wrapper.sent_first_chunk = _flag(state, "sent_first_chunk")
    wrapper.complete_response = _text(state.get("complete_response", ""))
    wrapper.completion_stream = _optional(state, "completion_stream")
    wrapper.send_stream_usage = _flag(state, "send_stream_usage")
    wrapper.is_function_call = _flag(state, "is_function_call")
    wrapper.tool_call = _flag(state, "tool_call")
    wrapper.response_id = _optional(state, "response_id")
    wrapper.created = _optional_int(state, "created")
    wrapper.system_fingerprint = _optional(state, "system_fingerprint")
    wrapper.chunks = [_model_response(item) for item in _list(state.get("chunks", []))]
    return wrapper


def _ascii(value: Json) -> bytes:
    return bytes(_int(item) for item in _list(value))


class _VertexChunk(SimpleNamespace):
    @property
    def text(self) -> str:
        candidates = cast("list[SimpleNamespace]", self.candidates)
        content = cast(SimpleNamespace, candidates[0].content)
        parts = cast("list[SimpleNamespace]", content.parts)
        texts = [
            value
            for value in (getattr(part, "text", None) for part in parts)
            if isinstance(value, str)
        ]
        if not texts:
            raise ValueError("Part has no text.")
        return "".join(texts)


def _vllm_chunk(value: Json) -> object:
    items: list[SimpleNamespace] = []
    for item in _list(value):
        outputs = [
            SimpleNamespace(**_object(output))
            for output in _list(_object(item).get("outputs", []))
        ]
        items.append(SimpleNamespace(outputs=outputs))
    return items


def _text_completion_chunk(value: Step) -> SimpleNamespace:
    choices = [
        SimpleNamespace(**_object(choice))
        for choice in _list(value.get("choices", []))
    ]
    usage = value.get("usage")
    if usage is None:
        return SimpleNamespace(choices=choices)
    counters = _object(usage)
    return SimpleNamespace(
        choices=choices,
        usage=SimpleNamespace(
            prompt_tokens=counters.get("prompt_tokens", 0),
            completion_tokens=counters.get("completion_tokens", 0),
            total_tokens=counters.get("total_tokens", 0),
        ),
    )


def _vertex_parts(value: Json) -> list[SimpleNamespace]:
    return [
        SimpleNamespace(**_object(part)) for part in _list(_object(value).get("parts", []))
    ]


def _vertex_chunk(value: Step) -> _VertexChunk:
    candidates: list[SimpleNamespace] = []
    for candidate in _list(value.get("candidates", [])):
        fields = _object(candidate)
        content = _object(fields.get("content", {}))
        candidates.append(
            SimpleNamespace(
                content=SimpleNamespace(parts=_vertex_parts(content)),
                finish_reason=SimpleNamespace(name=fields.get("finish_reason")),
            )
        )
    return _VertexChunk(candidates=candidates)


def _effective(step: Step, state: Step, key: str) -> Json:
    return step.get(key) if key in step else state.get(key)


def _chunk(step: Step, state: Step) -> object:
    spec = _object(step.get("chunk", {}))
    kind = _text(spec.get("kind", ""))
    value = spec.get("value")
    if kind == "text":
        return _text(value)
    if kind == "bytes":
        return _ascii(value)
    if kind == "generic":
        return dict(_object(value))
    return _json_chunk(step, state, value)


def _json_chunk(step: Step, state: Step, value: Json) -> object:
    custom = _effective(step, state, "custom_provider") is True
    provider = _effective(step, state, "provider")
    if provider == "vertex_ai" and isinstance(value, dict) and "candidates" in value:
        return _vertex_chunk(_object(value))
    if provider == "vllm" and isinstance(value, list):
        return _vllm_chunk(cast("Json", value))
    if (
        provider in ("text-completion-openai", "azure_text")
        and isinstance(value, dict)
        and "choices" in value
    ):
        return _text_completion_chunk(_object(value))
    if isinstance(value, dict) and (
        (custom and "choices" in value) or (not custom and provider in OPENAI_READERS)
    ):
        return _model_response(_object(value))
    return value


def _chunk_view(value: ModelResponseStream) -> Json:
    choices: list[Json] = [
        {
            "index": choice.index,
            "finish_reason": choice.finish_reason,
            "content": cast("Json", _attr(choice.delta, "content")),
        }
        for choice in value.choices
    ]
    return {"id": value.id, "choices": choices}


def _state_view(wrapper: CustomStreamWrapper) -> Json:
    return {
        "model": cast(str, wrapper.model),
        "received_finish_reason": wrapper.received_finish_reason,
        "intermittent_finish_reason": wrapper.intermittent_finish_reason,
        "completion_stream": cast("str | None", wrapper.completion_stream),
    }


def _usage_view(value: object) -> Json:
    if not isinstance(value, Usage):
        return None
    return {
        "prompt_tokens": value.prompt_tokens,
        "completion_tokens": value.completion_tokens,
        "total_tokens": value.total_tokens,
    }


def _reason(value: object) -> Json:
    return None if value is None or value == "" else cast("Json", value)


def _decode(wrapper: CustomStreamWrapper, step: Step, state: Step) -> Json:
    chunk = _chunk(step, state)
    model_response: ModelResponseStream = wrapper.model_response_creator()  # pyright: ignore[reportUnknownMemberType]
    completion_obj: dict[str, object] = {"content": ""}
    try:
        result = wrapper._dispatch_provider_chunk(chunk, model_response, completion_obj)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]
    except StopIteration:
        return {"raised": {"message": ""}}
    except Exception as error:  # noqa: BLE001 - the wire contract carries every reader exception
        return {"raised": {"message": str(error)}}
    if isinstance(result, _ProviderChunkEarlyReturn):
        early = result.value
        return {
            "early_return": True,
            "early_chunk": None if early is None else _chunk_view(early),
            "text": cast("Json", completion_obj.get("content")),
            "is_finished": False,
            "finish_reason": None,
            "usage": None,
            "tool_use": None,
            "provider_specific_fields": None,
            "tool_calls": None,
            "state": _state_view(wrapper),
        }
    response_obj = result.response_obj
    return {
        "early_return": False,
        "early_chunk": None,
        "text": cast("Json", completion_obj.get("content")),
        "is_finished": bool(response_obj.get("is_finished", False)),
        "finish_reason": _reason(response_obj.get("finish_reason")),
        "usage": _usage_view(_attr(model_response, "usage")),
        "tool_use": _raw(response_obj.get("tool_use")),
        "provider_specific_fields": _raw(response_obj.get("provider_specific_fields")),
        "tool_calls": _raw(completion_obj.get("tool_calls")),
        "state": _state_view(wrapper),
    }


def _function_call(delta: object) -> Json:
    function = _attr(delta, "function_call")
    if function is None:
        return None
    return {
        "name": cast("Json", _attr(function, "name")),
        "arguments": cast("Json", _attr(function, "arguments")),
    }


def _tool_calls(delta: object) -> Json:
    calls = _attr(delta, "tool_calls")
    if calls is None:
        return None
    listed: list[Json] = []
    for call in cast("list[object]", calls):
        function = _attr(call, "function")
        listed.append(
            {
                "id": cast("Json", _attr(call, "id")),
                "type": cast("Json", _attr(call, "type")),
                "function": {
                    "name": cast("Json", _attr(function, "name")),
                    "arguments": cast("Json", _attr(function, "arguments")),
                },
            }
        )
    return listed


def _summary(chunk: ModelResponseStream) -> Json:
    choice = chunk.choices[0] if chunk.choices else None
    delta = None if choice is None else choice.delta
    return {
        "id": chunk.id,
        "model": chunk.model,
        "created": chunk.created,
        "system_fingerprint": chunk.system_fingerprint,
        "finish_reason": None if choice is None else choice.finish_reason,
        "content": cast("Json", _attr(delta, "content")),
        "role": cast("Json", _attr(delta, "role")),
        "function_call": None if delta is None else _function_call(delta),
        "tool_calls": None if delta is None else _tool_calls(delta),
        "choices": len(chunk.choices),
        "usage": _usage_view(_attr(chunk, "usage")),
        "provider_specific_fields": _raw(chunk.provider_specific_fields),
    }


def _create(wrapper: CustomStreamWrapper, step: Step, state: Step) -> Json:
    chunk = _chunk(step, state)
    try:
        created = wrapper.chunk_creator(chunk)  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
    except StopIteration:
        return {"raised": {"message": ""}}
    except Exception as error:  # noqa: BLE001 - the wire contract carries every reader exception
        return {"raised": {"message": str(error)}}
    return {
        "chunk": None if created is None else _summary(created),
        "received_finish_reason": wrapper.received_finish_reason,
        "intermittent_finish_reason": wrapper.intermittent_finish_reason,
        "sent_first_chunk": wrapper.sent_first_chunk,
        "tool_call": wrapper.tool_call,
        "chunks": len(cast("list[object]", wrapper.chunks)),
    }


def _process(wrapper: CustomStreamWrapper, step: Step, _state: Step) -> Json:
    text = wrapper.process_chunk(_text(step.get("chunk", "")))
    return {"text": text, "complete_response": wrapper.complete_response}


_HANDLERS: dict[str, Adapter] = {
    "decode": _decode,
    "creator": _create,
    "process_chunk": _process,
}


def run(input_value: dict[str, Json]) -> Json:
    litellm.suppress_debug_info = True
    state = _object(input_value.get("state", {}))
    provider = _optional(state, "provider")
    if _flag(state, "custom_provider") and provider is not None:
        litellm._custom_providers.append(provider)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper = _wrapper(state)
    steps = _list(input_value.get("steps", []))
    return cast(
        "Json",
        [_run_step(wrapper, _object(step), state) for step in steps],
    )


def _run_step(wrapper: CustomStreamWrapper, step: Step, state: Step) -> Json:
    _apply_step(wrapper, step)
    op = _text(step.get("op", ""))
    handler = _HANDLERS.get(op)
    if handler is None:
        raise ValueError(f"unsupported operation {op}")
    return handler(wrapper, step, state)


def _apply_step(wrapper: CustomStreamWrapper, step: Step) -> None:
    if "provider" in step:
        wrapper.custom_llm_provider = _optional(step, "provider")
    if "model" in step:
        wrapper.model = _text(step["model"])
    if "send_stream_usage" in step:
        wrapper.send_stream_usage = _flag(step, "send_stream_usage")
    if "is_function_call" in step:
        wrapper.is_function_call = _flag(step, "is_function_call")
