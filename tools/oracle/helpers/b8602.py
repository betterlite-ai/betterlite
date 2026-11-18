from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.proxy import proxy_server
from litellm.proxy.proxy_server import (
    _format_fallback_metadata_sse_event,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9389
    _format_streaming_sse_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9603
    _get_client_requested_model_for_streaming,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9344
    _get_streaming_fallback_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9372
    _is_injected_stream_usage_artifact,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9563
    _is_positive_int_like,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9359
    _pop_complete_sse_frame,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9614
    _restamp_streaming_chunk_model,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9410
    _serialize_streaming_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9554
    _should_include_fallback_errors,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9366
)
from litellm.types.llms import openai as openai_types
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as CarrierBase
from litellm.types.utils import (
    ChatCompletionAudioResponse,
    Delta,
    ModelResponse,
    ModelResponseStream,
)
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
ViewName: TypeAlias = Literal["serialized", "usage_artifact", "text"]

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ModelCarrier(CarrierBase):
    model: str | None = None


_EVENT_TYPE: TypeAdapter[str] = TypeAdapter(str)

EVENT_CLASSES: Final[dict[str, type[CarrierBase]]] = {
    "error": openai_types.ErrorEvent,
    "response.content_part.added": openai_types.ContentPartAddedEvent,
    "response.content_part.done": openai_types.ContentPartDoneEvent,
    "response.completed": openai_types.ResponseCompletedEvent,
    "response.created": openai_types.ResponseCreatedEvent,
    "response.failed": openai_types.ResponseFailedEvent,
    "response.in_progress": openai_types.ResponseInProgressEvent,
    "response.incomplete": openai_types.ResponseIncompleteEvent,
    "response.output_item.added": openai_types.OutputItemAddedEvent,
    "response.output_item.done": openai_types.OutputItemDoneEvent,
    "response.reasoning_summary_part.added": openai_types.ResponsePartAddedEvent,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1619
    "response.reasoning_summary_part.done": openai_types.ReasoningSummaryPartDoneEvent,
}


def event_object(body: JsonObject) -> CarrierBase:
    event_class = EVENT_CLASSES.get(_EVENT_TYPE.validate_python(body["type"]))
    if event_class is None:
        return openai_types.GenericEvent.model_validate(body)
    return event_class.model_validate(body)


class FallbackView(TypedDict):
    fallback_model: str | None
    errors: list[JsonValue]


class EventPayload(TypedDict):
    id: str
    object: str
    created: int
    model: str
    choices: list[JsonValue]
    litellm_fallback: FallbackView


_PAYLOAD: TypeAdapter[EventPayload] = TypeAdapter(EventPayload)


class EventCase(TypedDict):
    model: str | None
    errors: list[JsonObject]
    created: int


class RestampCase(TypedDict):
    kind: Literal["stream", "full", "event", "plain"]
    body: JsonObject
    requested_model: str
    request_data: JsonObject
    fallback_attempted: bool
    fallback_model: str | None
    mismatch_logged: bool


class ChunkCase(TypedDict):
    kind: Literal["stream", "full", "event", "plain"]
    body: JsonObject


class Input(TypedDict):
    function: Literal[
        "client_model", "int_like", "include_errors", "fallback_meta",
        "fallback_event", "restamp", "chunk_view", "frames",
    ]
    requests: list[JsonObject]
    values: list[JsonValue]
    expose: bool
    hidden: list[JsonValue]
    events: list[EventCase]
    restamps: list[RestampCase]
    view: ViewName
    chunks: list[ChunkCase]
    payloads: list[str]
    buffers: list[str]
    endings: list[str]


class StreamChoice(Protocol):
    delta: Delta


ChunkObject: TypeAlias = ModelResponseStream | ModelResponse | ModelCarrier | JsonObject


def chunk_object(case: ChunkCase) -> ChunkObject:
    kind = case["kind"]
    body = case["body"]
    if kind == "plain":
        return dict(body)
    if kind == "event":
        return ModelCarrier.model_validate(body)
    if kind == "stream":
        chunk = ModelResponseStream(**body)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
        for choice in cast(list[StreamChoice], chunk.choices):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2006
            if choice.delta.audio is not None:
                choice.delta.audio = ChatCompletionAudioResponse.model_validate(choice.delta.audio)
        return chunk
    return ModelResponse(**body)  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181


def chunk_model(chunk: ChunkObject) -> JsonValue:
    match chunk:
        case ModelResponseStream() | ModelResponse() | ModelCarrier():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
            return chunk.model
        case dict():
            return chunk.get("model")


def restamp_answer(case: RestampCase) -> JsonValue:
    chunk = chunk_object(case)
    logged: bool = _restamp_streaming_chunk_model(  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9410
        chunk=chunk,
        requested_model_from_client=case["requested_model"],
        request_data=case["request_data"],
        model_mismatch_logged=case["mismatch_logged"],
        fallback_was_attempted=case["fallback_attempted"],
        fallback_model_from_metadata=case["fallback_model"],
    )[1]
    view: dict[str, JsonValue] = {"model": chunk_model(chunk), "logged": logged}
    return view


def client_models_answer(input_value: Input) -> JsonValue:
    return [
        _get_client_requested_model_for_streaming(request)
        for request in input_value["requests"]
    ]


def int_like_answer(input_value: Input) -> JsonValue:
    return [
        _is_positive_int_like(value)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9359
        for value in input_value["values"]
    ]


def include_errors_answer(input_value: Input) -> JsonValue:
    proxy_server.general_settings = {"expose_fallback_errors_to_caller": input_value["expose"]}  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9367
    return [
        _should_include_fallback_errors(request)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9366
        for request in input_value["requests"]
    ]


def fallback_meta_answer(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = []
    for hidden in input_value["hidden"]:
        attempted, model, errors = _get_streaming_fallback_metadata({"_hidden_params": hidden})
        view: dict[str, JsonValue] = {"attempted": attempted, "model": model}
        view["errors"] = [_JSON.validate_python(error) for error in errors]
        answers.append(view)
    return answers


def fallback_events_answer(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = []
    for case in input_value["events"]:
        frame = _format_fallback_metadata_sse_event(
            fallback_model=case["model"],
            fallback_errors=case["errors"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9389
        )
        payload: EventPayload = _PAYLOAD.validate_json(
            frame.removeprefix("data: ").removesuffix("\n\n")
        )
        payload["created"] = case["created"]
        view: dict[str, JsonValue] = {
            "payload": _JSON.validate_python(payload),
            "prefix": frame.startswith("data: "),
            "suffix": frame.endswith("\n\n"),
        }
        answers.append(view)
    return answers


def restamps_answer(input_value: Input) -> JsonValue:
    return [restamp_answer(case) for case in input_value["restamps"]]


def serialized_chunk(case: ChunkCase) -> JsonValue:
    if case["kind"] != "event":
        chunk = chunk_object(case)
        match chunk:
            case dict():
                return _JSON.validate_python(chunk)
            case ModelResponseStream() | ModelResponse() | ModelCarrier():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9554
                return _JSON.validate_json(_serialize_streaming_chunk(chunk))
    return _JSON.validate_json(_serialize_streaming_chunk(event_object(case["body"])))


def usage_chunk(case: ChunkCase) -> JsonValue:
    chunk = chunk_object(case)
    return _is_injected_stream_usage_artifact(chunk)


def text_chunk(case: ChunkCase) -> JsonValue:
    chunk = chunk_object(case)
    match chunk:
        case ModelResponseStream() | ModelResponse():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5389
            return litellm.get_response_string(response_obj=chunk)
        case ModelCarrier() | dict():
            return ""


VIEWS: Final[dict[ViewName, Callable[[ChunkCase], JsonValue]]] = {
    "serialized": serialized_chunk,
    "usage_artifact": usage_chunk,
    "text": text_chunk,
}


def chunk_views_answer(input_value: Input) -> JsonValue:
    return [VIEWS[input_value["view"]](case) for case in input_value["chunks"]]


def sse_frame(payload: str) -> str:
    frame = _format_streaming_sse_chunk(payload)
    match frame:
        case str():
            return frame
        case bytes():
            return frame.decode("utf-8")


def frames_answer(input_value: Input) -> JsonValue:
    splits: list[JsonValue] = []
    for buffer in input_value["buffers"]:
        frame, rest = _pop_complete_sse_frame(buffer)
        split: dict[str, JsonValue] = {"frame": frame, "rest": rest}
        splits.append(split)
    ends: list[JsonValue] = [
        text.endswith(proxy_server._SSE_FRAME_DELIMITERS)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9609
        for text in input_value["endings"]
    ]
    data_frames: list[JsonValue] = [
        sse_frame(payload) for payload in input_value["payloads"]
    ]
    answer: dict[str, JsonValue] = {
        "data_frames": data_frames,
        "splits": splits,
        "ends": ends,
    }
    return answer


HANDLERS: Final[dict[str, Callable[[Input], JsonValue]]] = {
    "client_model": client_models_answer,
    "int_like": int_like_answer,
    "include_errors": include_errors_answer,
    "fallback_meta": fallback_meta_answer,
    "fallback_event": fallback_events_answer,
    "restamp": restamps_answer,
    "chunk_view": chunk_views_answer,
    "frames": frames_answer,
}


def run(input_value: Input) -> JsonValue:
    handler = HANDLERS[input_value["function"]]
    return handler(input_value)
