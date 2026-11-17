from typing import Literal, NotRequired, TypedDict

from litellm.exceptions import MidStreamFallbackError
from litellm.proxy.common_utils.responses_stream_errors import ResponsesStreamErrorState
from pydantic import JsonValue, TypeAdapter

FRAME_PREFIX = "event: response.failed\ndata: "
FRAME_SUFFIX = "\n\n"
MASKED_ID = "resp_00000000000000000000000000000000"
MASKED_TIME = -999999999


class FailureSpec(TypedDict):
    kind: Literal["plain", "mid_stream_fallback"]
    text: str
    message: JsonValue
    type: JsonValue
    code: JsonValue
    status_code: JsonValue
    body: JsonValue
    original_exception: "FailureSpec | None"


class ObserveStep(TypedDict):
    function: Literal["observe_chunk"]
    chunk: JsonValue


class MarkEmittedStep(TypedDict):
    function: Literal["mark_emitted"]
    frame: str


class FormatFailureStep(TypedDict):
    function: Literal["format_failure"]
    failure: FailureSpec


Step = ObserveStep | MarkEmittedStep | FormatFailureStep


class Input(TypedDict):
    redaction: Literal["enabled", "disabled"]
    steps: list[Step]


class FrameError(TypedDict):
    code: str
    message: str


class FrameResponse(TypedDict):
    id: str
    created_at: int
    error: FrameError
    model: NotRequired[str]
    object: Literal["response"]
    output: list[JsonValue]
    status: Literal["failed"]


class FrameEvent(TypedDict):
    type: Literal["response.failed"]
    response: FrameResponse
    sequence_number: int


class ObserveResult(TypedDict):
    function: Literal["observe_chunk"]


class MarkEmittedResult(TypedDict):
    function: Literal["mark_emitted"]
    frame: str


class FormatFailureResult(TypedDict):
    function: Literal["format_failure"]
    text: str | None


Result = ObserveResult | MarkEmittedResult | FormatFailureResult


class StateView(TypedDict):
    response_id: str | None
    model: str | None
    created_at: int | None
    sequence_number: int
    terminal_emitted: bool


class Output(TypedDict):
    results: list[Result]
    state: StateView


class StreamFailureError(Exception):
    def __init__(self, spec: FailureSpec) -> None:
        super().__init__(spec["text"])
        self.message: JsonValue = spec["message"]
        self.code: JsonValue = spec["code"]
        self.type: JsonValue = spec["type"]
        self.status_code: JsonValue = spec["status_code"]
        self.body: JsonValue = spec["body"]


FRAME: TypeAdapter[FrameEvent] = TypeAdapter(FrameEvent)


def build_failure(spec: FailureSpec) -> Exception:
    original = spec["original_exception"]
    if spec["kind"] == "plain":
        return StreamFailureError(spec)
    return MidStreamFallbackError(
        message=spec["text"],
        model="oracle-model",
        llm_provider="openai",
        original_exception=None if original is None else build_failure(original),
    )


def mask_frame(frame: str, *, id_committed: bool, time_committed: bool) -> str:
    if not frame.startswith(FRAME_PREFIX) or not frame.endswith(FRAME_SUFFIX):
        return frame
    event = FRAME.validate_json(frame.removeprefix(FRAME_PREFIX).removesuffix(FRAME_SUFFIX))
    masked = frame
    if not id_committed:
        masked = masked.replace(event["response"]["id"], MASKED_ID)
    if not time_committed:
        masked = masked.replace(str(event["response"]["created_at"]), str(MASKED_TIME))
    return masked


def state_view(state: ResponsesStreamErrorState) -> StateView:
    return {
        "response_id": state.response_id,
        "model": state.model,
        "created_at": state.created_at,
        "sequence_number": state.sequence_number,
        "terminal_emitted": state.terminal_emitted,
    }


def run(input_value: Input) -> Output:
    state = ResponsesStreamErrorState()
    results: list[Result] = []
    for step in input_value["steps"]:
        if step["function"] == "observe_chunk":
            state.observe_chunk(step["chunk"])
            results.append({"function": "observe_chunk"})
        elif step["function"] == "mark_emitted":
            emitted = str(state.mark_emitted(step["frame"]))
            results.append({"function": "mark_emitted", "frame": emitted})
        else:
            frame = state.format_failure(build_failure(step["failure"]))
            id_committed = state.response_id is not None and state.response_id != ""
            time_committed = state.created_at is not None
            text = (
                None
                if frame is None
                else mask_frame(frame, id_committed=id_committed, time_committed=time_committed)
            )
            results.append({"function": "format_failure", "text": text})
    return {"results": results, "state": state_view(state)}
