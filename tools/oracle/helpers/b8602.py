from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy.proxy_server import (
    _format_fallback_metadata_sse_event,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9389
    _format_streaming_sse_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9603
    _get_client_requested_model_for_streaming,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9344
    _get_streaming_fallback_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9372
    _is_injected_stream_usage_artifact,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9563
    _is_positive_int_like,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9359
    _pop_complete_sse_frame,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9614
    _restamp_streaming_chunk_model,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9410
    _serialize_streaming_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9554
    _should_include_fallback_errors,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9366
)
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
ViewName: TypeAlias = Literal["serialized", "usage_artifact", "text"]

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ModelCarrier(BaseLiteLLMOpenAIResponseObject):
    model: str | None = None


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


class ClientModelInput(TypedDict):
    function: Literal["client_model"]
    requests: list[JsonObject]


class IntLikeInput(TypedDict):
    function: Literal["int_like"]
    values: list[JsonValue]


class IncludeErrorsInput(TypedDict):
    function: Literal["include_errors"]
    expose: bool
    requests: list[JsonObject]


class FallbackMetaInput(TypedDict):
    function: Literal["fallback_meta"]
    hidden: list[JsonValue]


class EventCase(TypedDict):
    model: str | None
    errors: list[JsonObject]
    created: int


class FallbackEventInput(TypedDict):
    function: Literal["fallback_event"]
    events: list[EventCase]


class RestampCase(TypedDict):
    kind: Literal["stream", "full", "event", "plain"]
    body: JsonObject
    requested_model: str
    request_data: JsonObject
    fallback_attempted: bool
    fallback_model: str | None
    mismatch_logged: bool


class RestampInput(TypedDict):
    function: Literal["restamp"]
    restamps: list[RestampCase]


class ChunkCase(TypedDict):
    kind: Literal["stream", "full", "event", "plain"]
    body: JsonObject


class ChunkViewInput(TypedDict):
    function: Literal["chunk_view"]
    view: ViewName
    chunks: list[ChunkCase]


class FramesInput(TypedDict):
    function: Literal["frames"]
    payloads: list[str]
    buffers: list[str]
    endings: list[str]


Input: TypeAlias = (
    ClientModelInput
    | IntLikeInput
    | IncludeErrorsInput
    | FallbackMetaInput
    | FallbackEventInput
    | RestampInput
    | ChunkViewInput
    | FramesInput
)

ChunkObject: TypeAlias = ModelResponseStream | ModelResponse | ModelCarrier | JsonObject


def chunk_object(case: ChunkCase) -> ChunkObject:
    kind = case["kind"]
    body = case["body"]
    if kind == "stream":
        return ModelResponseStream(**body)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    if kind == "full":
        return ModelResponse(**body)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181
    if kind == "event":
        return ModelCarrier.model_validate(body)
    return dict(body)


def chunk_model(chunk: ChunkObject) -> JsonValue:
    match chunk:
        case ModelResponseStream() | ModelResponse():
            return chunk.model
        case ModelCarrier():
            return chunk.model
        case dict():
            return chunk.get("model")


def restamp_call(chunk: ChunkObject, case: RestampCase) -> bool:
    logged: bool = _restamp_streaming_chunk_model(chunk=chunk, requested_model_from_client=case["requested_model"], request_data=case["request_data"], model_mismatch_logged=case["mismatch_logged"], fallback_was_attempted=case["fallback_attempted"], fallback_model_from_metadata=case["fallback_model"])[1]  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9410
    return logged


def restamp_answer(case: RestampCase) -> JsonValue:
    chunk = chunk_object(case)
    logged = restamp_call(chunk, case)
    view: dict[str, JsonValue] = {"model": chunk_model(chunk), "logged": logged}
    return view


def client_models_answer(input_value: ClientModelInput) -> JsonValue:
    answers: list[JsonValue] = []
    answers.extend(
        _get_client_requested_model_for_streaming(request)
        for request in input_value["requests"]
    )
    return answers


def int_like_answer(input_value: IntLikeInput) -> JsonValue:
    answers: list[JsonValue] = []
    answers.extend(_is_positive_int_like(raw) for raw in input_value["values"])  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9359
    return answers


def include_errors_answer(input_value: IncludeErrorsInput) -> JsonValue:
    proxy_server.general_settings = {
        "expose_fallback_errors_to_caller": input_value["expose"]
    }
    answers: list[JsonValue] = []
    answers.extend(_should_include_fallback_errors(request) for request in input_value["requests"])  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9366
    return answers


def fallback_meta_answer(input_value: FallbackMetaInput) -> JsonValue:
    answers: list[JsonValue] = []
    for hidden in input_value["hidden"]:
        attempted, model, errors = _get_streaming_fallback_metadata(
            {"_hidden_params": hidden}
        )
        view: dict[str, JsonValue] = {"attempted": attempted, "model": model}
        view["errors"] = [_JSON.validate_python(error) for error in errors]
        answers.append(view)
    return answers


def fallback_events_answer(input_value: FallbackEventInput) -> JsonValue:
    answers: list[JsonValue] = []
    for case in input_value["events"]:
        frame = _format_fallback_metadata_sse_event(fallback_model=case["model"], fallback_errors=case["errors"])  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9389
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


def restamps_answer(input_value: RestampInput) -> JsonValue:
    answers: list[JsonValue] = []
    answers.extend(restamp_answer(case) for case in input_value["restamps"])
    return answers


def serialized_chunk(case: ChunkCase) -> JsonValue:
    chunk = chunk_object(case)
    text: str | bytes = ""
    match chunk:
        case ModelResponseStream():
            text = _serialize_streaming_chunk(chunk)
        case ModelResponse():
            text = _serialize_streaming_chunk(chunk)
        case ModelCarrier():
            text = _serialize_streaming_chunk(chunk)
        case dict():
            return _JSON.validate_python(chunk)
    return _JSON.validate_json(text)


def usage_chunk(case: ChunkCase) -> JsonValue:
    chunk = chunk_object(case)
    return _is_injected_stream_usage_artifact(chunk)


def text_chunk(case: ChunkCase) -> JsonValue:
    chunk = chunk_object(case)
    match chunk:
        case ModelResponseStream() | ModelResponse():
            return litellm.get_response_string(response_obj=chunk)
        case ModelCarrier():
            return ""
        case dict():
            return ""


def chunk_view(case: ChunkCase, view: ViewName) -> JsonValue:
    if view == "serialized":
        return serialized_chunk(case)
    if view == "usage_artifact":
        return usage_chunk(case)
    return text_chunk(case)


def chunk_views_answer(input_value: ChunkViewInput) -> JsonValue:
    view = input_value["view"]
    answers: list[JsonValue] = []
    answers.extend(chunk_view(case, view) for case in input_value["chunks"])
    return answers


def sse_frame(payload: str) -> str:
    frame: str | bytes = _format_streaming_sse_chunk(payload)
    match frame:
        case str():
            return frame
        case bytes():
            return frame.decode("utf-8")


def frames_answer(input_value: FramesInput) -> JsonValue:
    splits: list[JsonValue] = []
    for buffer in input_value["buffers"]:
        frame, rest = _pop_complete_sse_frame(buffer)
        view: dict[str, JsonValue] = {"frame": frame, "rest": rest}
        splits.append(view)
    ends: list[JsonValue] = []
    ends.extend(text.endswith(proxy_server._SSE_FRAME_DELIMITERS) for text in input_value["endings"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L9609
    data_frames: list[JsonValue] = []
    data_frames.extend(sse_frame(payload) for payload in input_value["payloads"])
    answer: dict[str, JsonValue] = {
        "data_frames": data_frames,
        "splits": splits,
        "ends": ends,
    }
    return answer


def run(input_value: Input) -> JsonValue:
    answer: JsonValue
    if input_value["function"] == "client_model":
        answer = client_models_answer(input_value)
    elif input_value["function"] == "int_like":
        answer = int_like_answer(input_value)
    elif input_value["function"] == "include_errors":
        answer = include_errors_answer(input_value)
    elif input_value["function"] == "fallback_meta":
        answer = fallback_meta_answer(input_value)
    elif input_value["function"] == "fallback_event":
        answer = fallback_events_answer(input_value)
    elif input_value["function"] == "restamp":
        answer = restamps_answer(input_value)
    elif input_value["function"] == "chunk_view":
        answer = chunk_views_answer(input_value)
    else:
        answer = frames_answer(input_value)
    return answer
