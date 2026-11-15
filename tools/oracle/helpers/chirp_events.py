from typing import Literal, TypedDict, cast

import litellm
from litellm.llms.vertex_ai.audio_transcription import realtime_transformation
from litellm.types.llms import vertex_ai_speech_to_text
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

_JSON: TypeAdapter[Json] = TypeAdapter(Json)


class PunctuationPair(TypedDict):
    previous: str
    current: str


class PunctuationInput(TypedDict):
    function: Literal["punctuation"]
    pairs: list[PunctuationPair]


class SessionDocument(TypedDict):
    model: str
    language: str | None
    sample_rate: int
    server_vad: bool


class ResultDocument(TypedDict):
    transcript: str
    is_final: bool


class ResponseFrame(TypedDict):
    kind: Literal["response"]
    speech_event: Literal["none", "begin", "end"]
    results: list[ResultDocument]
    billed_seconds: float


class ConfiguredFrame(TypedDict):
    kind: Literal["configured"]


class FinishedFrame(TypedDict):
    kind: Literal["turn_finished"]


class DiscardedFrame(TypedDict):
    kind: Literal["turn_discarded"]
    billed_seconds: float


Frame = ResponseFrame | ConfiguredFrame | FinishedFrame | DiscardedFrame


class TraceInput(TypedDict):
    function: Literal["trace"]
    session_id: str
    session: SessionDocument
    frames: list[Frame]


class SessionsInput(TypedDict):
    function: Literal["sessions"]
    expected_model: str
    payloads: list[str]


class FixedItemIds:
    position: int

    def __init__(self) -> None:
        self.position = 0

    def next_item_id(self) -> str:
        identifier = f"item_{self.position}"
        self.position += 1
        return identifier


def _frame(frame: Frame) -> vertex_ai_speech_to_text.VertexSpeechStreamingEventUnion:
    if frame["kind"] == "response":
        event: vertex_ai_speech_to_text.VertexSpeechStreamingEventUnion = (
            vertex_ai_speech_to_text.VertexSpeechStreamingResponse(
                speech_event=frame["speech_event"],
                results=tuple(
                    vertex_ai_speech_to_text.VertexSpeechStreamingResult(
                        transcript=result["transcript"],
                        is_final=result["is_final"],
                    )
                    for result in frame["results"]
                ),
                billed_seconds=frame["billed_seconds"],
            )
        )
    elif frame["kind"] == "configured":
        event = vertex_ai_speech_to_text.VertexSpeechStreamingConfigured()
    elif frame["kind"] == "turn_finished":
        event = vertex_ai_speech_to_text.VertexSpeechStreamingTurnFinished()
    else:
        event = vertex_ai_speech_to_text.VertexSpeechStreamingTurnDiscarded(
            billed_seconds=frame["billed_seconds"]
        )
    return event


def _punctuation(input_value: PunctuationInput) -> Json:
    answers: list[Json] = [
        realtime_transformation.new_words(pair["previous"], pair["current"])
        for pair in input_value["pairs"]
    ]
    document: dict[str, Json] = {"answers": answers}
    return _JSON.validate_python(document)


def _trace(input_value: TraceInput) -> Json:
    session = input_value["session"]
    config = realtime_transformation.ChirpSessionConfig(
        model=session["model"],
        language=session["language"],
        sample_rate=session["sample_rate"],
        server_vad=session["server_vad"],
    )
    item_ids = FixedItemIds()
    transformer = realtime_transformation.ChirpEventTransformer(
        new_item_id=item_ids.next_item_id
    )
    transformer.configure(config, input_value["session_id"])
    events: list[Json] = []
    for frame in input_value["frames"]:
        emitted = cast("tuple[object, ...]", transformer.transform(_frame(frame)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/audio_transcription/realtime_transformation.py#L230
        events.extend(_JSON.validate_python(event) for event in emitted)
    usage = transformer.take_unbilled_usage()
    repeated_usage = transformer.take_unbilled_usage()
    usage_document = None if usage is None else _JSON.validate_python(usage)
    repeated_document = None if repeated_usage is None else _JSON.validate_python(repeated_usage)
    document: dict[str, Json] = {
        "events": events,
        "usage": usage_document,
        "repeated_usage": repeated_document,
    }
    return _JSON.validate_python(document)


def _session_document(payload: str, expected_model: str) -> Json:
    try:
        config = realtime_transformation.parse_chirp_session_update(payload, expected_model)
    except realtime_transformation.ChirpProtocolError:
        return None
    document: dict[str, Json] = {
        "model": config.model,
        "language": config.language,
        "sample_rate": config.sample_rate,
        "server_vad": config.server_vad,
    }
    return _JSON.validate_python(document)


def _sessions(input_value: SessionsInput) -> Json:
    sessions = [
        _session_document(payload, input_value["expected_model"])
        for payload in input_value["payloads"]
    ]
    document: dict[str, Json] = {"sessions": sessions}
    return _JSON.validate_python(document)


def run(input_value: PunctuationInput | TraceInput | SessionsInput) -> Json:
    if input_value["function"] == "punctuation":
        return _punctuation(input_value)
    if input_value["function"] == "trace":
        return _trace(input_value)
    return _sessions(input_value)
