import importlib.util
import json
import sys
from collections.abc import Callable
from types import ModuleType, SimpleNamespace
from typing import TypeAlias, cast

import litellm
from litellm.litellm_core_utils.redact_messages import LiteLLMLoggingObject
from litellm.litellm_core_utils.streaming_handler import (
    CustomStreamWrapper,
    _ProviderChunkEarlyReturn,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.utils import BaseModel, ModelResponseStream, Usage
from openai._models import construct_type  # pyright: ignore[reportPrivateUsage]
from openai.types import Completion
from openai.types.chat import ChatCompletionChunk

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Step: TypeAlias = dict[str, Json]

OPENAI_READERS: frozenset[str] = frozenset({"openai", "azure", "azure_ai", "cached_response"})
SDK_READERS: frozenset[str] = frozenset({"openai", "azure"})


def _obj(value: Json) -> dict[str, Json]:
    return cast("dict[str, Json]", value)


def _list(value: Json) -> list[Json]:
    return cast("list[Json]", value)


def _txt(value: Json) -> str:
    return cast(str, value)


def _opt(state: Step, key: str) -> str | None:
    value = state.get(key)
    return None if value is None else _txt(value)


def _flag(state: Step, key: str) -> bool:
    return state.get(key) is True


def _attr(value: object, key: str) -> object:
    return getattr(value, key, None)


def _raw(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast("Json", value.model_dump())
    return cast("Json", json.loads(json.dumps(value)))


def _model_response(value: Json) -> ModelResponseStream:
    data = dict(_obj(value))
    hidden = data.pop("hidden_params", None)
    response = ModelResponseStream(**data)  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]
    response._hidden_params = {} if hidden is None else dict(_obj(hidden))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    return response


def _sdk_chunk(value: Json) -> ChatCompletionChunk:
    data = {"id": "", "object": "chat.completion.chunk", "created": 0, **_obj(value)}
    hidden = data.pop("hidden_params", None)
    chunk = cast(ChatCompletionChunk, construct_type(type_=ChatCompletionChunk, value=data))
    if hidden is not None:
        chunk._hidden_params = dict(_obj(hidden))  # noqa: SLF001  # pyright: ignore[reportAttributeAccessIssue]
    return chunk


def _wrapper(state: Step) -> CustomStreamWrapper:
    provider = _opt(state, "provider")
    logging = SimpleNamespace(optional_params={}, messages=None)
    logging.model_call_details = {"litellm_params": {}, "custom_llm_provider": provider}
    wrapper = CustomStreamWrapper(
        completion_stream=[],
        model=_txt(state.get("model", "")),
        logging_obj=cast(LiteLLMLoggingObject, logging),
        custom_llm_provider=provider,
    )
    wrapper._base_hidden_params = dict(_obj(state.get("base_hidden_params", {})))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper.received_finish_reason = _opt(state, "received_finish_reason")
    wrapper.intermittent_finish_reason = _opt(state, "intermittent_finish_reason")
    wrapper.sent_first_chunk = _flag(state, "sent_first_chunk")
    wrapper.complete_response = _txt(state.get("complete_response", ""))
    wrapper.completion_stream = _opt(state, "completion_stream")
    wrapper.send_stream_usage = _flag(state, "send_stream_usage")
    wrapper.is_function_call = _flag(state, "is_function_call")
    wrapper.tool_call = _flag(state, "tool_call")
    wrapper.response_id = _opt(state, "response_id")
    created = state.get("created")
    wrapper.created = None if created is None else cast(int, created)
    wrapper.system_fingerprint = _opt(state, "system_fingerprint")
    wrapper.chunks = [_model_response(item) for item in _list(state.get("chunks", []))]
    return wrapper


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


def _vertex_part(part: dict[str, Json]) -> SimpleNamespace:
    call = part.get("function_call")
    if call is None:
        return SimpleNamespace(**part)
    fields = _obj(call)
    function = SimpleNamespace(name=fields.get("name"), args=_obj(fields.get("args", {})))
    return SimpleNamespace(**{**part, "function_call": function})


def _vertex_chunk(value: Step) -> _VertexChunk:
    candidates = [
        SimpleNamespace(
            content=SimpleNamespace(
                parts=[
                    _vertex_part(_obj(part))
                    for part in _list(_obj(_obj(candidate).get("content", {})).get("parts", []))
                ]
            ),
            finish_reason=SimpleNamespace(
                name=_obj(candidate).get("finish_reason", "FINISH_REASON_UNSPECIFIED")
            ),
        )
        for candidate in _list(value.get("candidates", []))
    ]
    return _VertexChunk(candidates=candidates)


def _vllm_chunk(value: list[Json]) -> list[SimpleNamespace]:
    return [
        SimpleNamespace(
            outputs=[
                SimpleNamespace(**_obj(output)) for output in _list(_obj(item).get("outputs", []))
            ]
        )
        for item in value
    ]


def _completion_chunk(value: Step) -> Completion:
    data = {"id": "", "object": "text_completion", "created": 0, "model": "", **value}
    return cast(Completion, construct_type(type_=Completion, value=data))


_DICT_READERS: dict[str, tuple[str, Callable[[Step], object]]] = {
    "vertex_ai": ("candidates", _vertex_chunk),
    "text-completion-openai": ("choices", _completion_chunk),
    "azure_text": ("choices", _completion_chunk),
}


def _json_chunk(step: Step, state: Step, value: Json) -> object:
    custom = step.get("custom_provider", state.get("custom_provider")) is True
    provider = step.get("provider", state.get("provider"))
    if provider == "vllm" and isinstance(value, list):
        return _vllm_chunk(value)
    if not isinstance(value, dict):
        return value
    reader = _DICT_READERS.get(cast(str, provider))
    if reader is not None and reader[0] in value:
        return reader[1](value)
    if not custom and provider in SDK_READERS:
        return _sdk_chunk(value)
    if (custom and "choices" in value) or (not custom and provider in OPENAI_READERS):
        return _model_response(value)
    return value


def _chunk(step: Step, state: Step) -> object:
    spec = _obj(step.get("chunk", {}))
    kind = _txt(spec.get("kind", ""))
    value = spec.get("value")
    if kind == "text":
        return _txt(value)
    if kind == "bytes":
        return bytes(cast("list[int]", value))
    if kind == "generic":
        return dict(_obj(value))
    return _json_chunk(step, state, value)


def _usage(value: object) -> Json:
    if not isinstance(value, Usage):
        return None
    return [
        value.prompt_tokens,
        value.completion_tokens,
        value.total_tokens,
        cast("int | None", _attr(value.prompt_tokens_details, "cached_tokens")),
        cast("int | None", _attr(value.completion_tokens_details, "reasoning_tokens")),
    ]


def _summary(chunk: ModelResponseStream) -> list[Json]:
    choice = chunk.choices[0] if chunk.choices else None
    delta = None if choice is None else choice.delta
    function = None if delta is None else _attr(delta, "function_call")
    calls = None if delta is None else _attr(delta, "tool_calls")
    listed: Json = None
    if calls is not None:
        listed = cast(
            "Json",
            [
                [
                    cast("Json", _attr(call, "id")),
                    cast("Json", _attr(call, "type")),
                    [
                        cast("Json", _attr(_attr(call, "function"), "name")),
                        cast("Json", _attr(_attr(call, "function"), "arguments")),
                    ],
                    cast("Json", _attr(call, "index")),
                ]
                for call in cast("list[object]", calls)
            ],
        )
    return [
        chunk.id,
        chunk.model,
        chunk.created,
        chunk.system_fingerprint,
        None if choice is None else choice.finish_reason,
        None if delta is None else _raw(_attr(delta, "content")),
        None if delta is None else _raw(_attr(delta, "role")),
        None
        if function is None
        else [cast("Json", _attr(function, "name")), cast("Json", _attr(function, "arguments"))],
        listed,
        len(chunk.choices),
        _usage(_attr(chunk, "usage")),
        _raw(chunk.provider_specific_fields),
    ]


def _state_line(wrapper: CustomStreamWrapper) -> list[Json]:
    return [
        cast(str, wrapper.model),
        wrapper.received_finish_reason,
        wrapper.intermittent_finish_reason,
        cast("str | None", wrapper.completion_stream),
    ]


def _decode(wrapper: CustomStreamWrapper, step: Step, state: Step) -> Json:
    chunk = _chunk(step, state)
    model_response = wrapper.model_response_creator()  # pyright: ignore[reportUnknownMemberType]
    completion_obj: dict[str, object] = {"content": ""}
    result = wrapper._dispatch_provider_chunk(chunk, model_response, completion_obj)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]
    early = result.value if isinstance(result, _ProviderChunkEarlyReturn) else None
    response: dict[str, object] = (
        {} if isinstance(result, _ProviderChunkEarlyReturn) else result.response_obj
    )
    reason = response.get("finish_reason")
    return cast(
        "Json",
        [
            completion_obj.get("content"),
            bool(response.get("is_finished", False)),
            None if reason is None or reason == "" else reason,
            _usage(_attr(model_response, "usage")),
            _raw(response.get("tool_use")),
            _raw(response.get("provider_specific_fields")),
            _raw(completion_obj.get("tool_calls")),
            isinstance(result, _ProviderChunkEarlyReturn),
            None if early is None else early.id,
            _state_line(wrapper),
        ],
    )


def _create(wrapper: CustomStreamWrapper, step: Step, state: Step) -> Json:
    created = wrapper.chunk_creator(_chunk(step, state))  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
    return cast(
        "Json",
        [
            None if created is None else _summary(created),
            wrapper.received_finish_reason,
            wrapper.intermittent_finish_reason,
            wrapper.sent_first_chunk,
            wrapper.tool_call,
            len(cast("list[object]", wrapper.chunks)),
        ],
    )


def _process(wrapper: CustomStreamWrapper, step: Step, _state: Step) -> Json:
    text = wrapper.process_chunk(_txt(step.get("chunk", "")))
    return [text, cast("Json", wrapper.complete_response)]


_HANDLERS = {"decode": _decode, "creator": _create, "process_chunk": _process}


def _apply(wrapper: CustomStreamWrapper, step: Step) -> None:
    if "provider" in step:
        wrapper.custom_llm_provider = _opt(step, "provider")
    if "model" in step:
        wrapper.model = _txt(step["model"])
    if "send_stream_usage" in step:
        wrapper.send_stream_usage = _flag(step, "send_stream_usage")
    if "is_function_call" in step:
        wrapper.is_function_call = _flag(step, "is_function_call")


def _provide_proto() -> None:
    if importlib.util.find_spec("proto") is None:
        repeated = SimpleNamespace(RepeatedComposite=type("RepeatedComposite", (), {}))
        marshal = SimpleNamespace(collections=SimpleNamespace(repeated=repeated))
        sys.modules["proto"] = cast(ModuleType, SimpleNamespace(marshal=marshal))


def run(input_value: dict[str, Json]) -> Json:
    litellm.suppress_debug_info = True
    _provide_proto()
    state = _obj(input_value.get("state", {}))
    provider = _opt(state, "provider")
    if _flag(state, "custom_provider") and provider is not None:
        litellm._custom_providers.append(provider)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    wrapper = _wrapper(state)
    outputs: list[Json] = []
    for raw in _list(input_value.get("steps", [])):
        step = _obj(raw)
        _apply(wrapper, step)
        op = _txt(step.get("op", ""))
        handler = _HANDLERS.get(op)
        if handler is None:
            raise ValueError(f"unsupported operation {op}")
        try:
            outputs.append(handler(wrapper, step, state))
        except StopIteration:
            outputs.append({"raised": {"message": ""}})
        except Exception as error:  # noqa: BLE001 - the wire contract carries every reader exception
            outputs.append({"raised": {"message": str(error)}})
    return outputs
