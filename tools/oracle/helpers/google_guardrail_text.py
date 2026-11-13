import asyncio
from collections.abc import Sequence
from copy import deepcopy
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.google_genai.guardrail_translation.handler import (
    GoogleGenAIGenerateContentHandler,
)
from pydantic import JsonValue, TypeAdapter

Record = dict[str, JsonValue]
Replacement = list[str]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class GuardrailInputs(TypedDict):
    texts: list[str]
    model: NotRequired[str | None]


class GuardrailLike(Protocol):
    async def apply_guardrail(
        self,
        inputs: GuardrailInputs,
        request_data: Record,
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> GuardrailInputs: ...


class GoogleHandler(Protocol):
    async def process_input_messages(
        self,
        data: Record,
        guardrail_to_apply: GuardrailLike,
        litellm_logging_obj: Logging | None = None,
    ) -> None: ...

    async def process_output_response(
        self,
        response: Record,
        guardrail_to_apply: GuardrailLike,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: Record | None = None,
        request_data: Record | None = None,
    ) -> None: ...

    async def process_output_streaming_response(  # ast-grep-ignore: no-long-names
        self,
        responses_so_far: Sequence[JsonValue | bytes],
        guardrail_to_apply: GuardrailLike,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: Record | None = None,
        request_data: Record | None = None,
    ) -> None: ...


# LiteLLM declares dict and UserAPIKeyAuth, while its code accepts plain dicts
HANDLER: GoogleHandler = cast(GoogleHandler, GoogleGenAIGenerateContentHandler())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/google_genai/guardrail_translation/handler.py#L158-L255


class RecordedCall(TypedDict):
    texts: list[str]
    model: str | None
    input_type: str
    request_data: Record


class CallResult(TypedDict):
    called: bool
    call_count: int
    texts: list[str] | None
    model: str | None
    input_type: str | None
    request_data: Record | None
    data: JsonValue


class SseResult(TypedDict):
    text: str | None


class RunResult(TypedDict):
    calls: list[JsonValue]


class RequestCall(TypedDict):
    function: Literal["request"]
    data: Record
    replacement: NotRequired[Replacement]


class ResponseCall(TypedDict):
    function: Literal["response"]
    response: Record
    replacement: NotRequired[Replacement]
    request_data: NotRequired[Record]
    user_api_key_dict: NotRequired[Record]


class StreamCall(TypedDict):
    function: Literal["stream"]
    frames: list[JsonValue]
    replacement: NotRequired[Replacement]
    request_data: NotRequired[Record]
    user_api_key_dict: NotRequired[Record]


class TextChunk(TypedDict):
    kind: Literal["text"]
    value: str


class ByteChunk(TypedDict):
    kind: Literal["bytes"]
    value: list[int]


SseChunk = TextChunk | ByteChunk


class SseCall(TypedDict):
    function: Literal["sse"]
    chunks: list[SseChunk]


Call = RequestCall | ResponseCall | StreamCall | SseCall


class Input(TypedDict):
    calls: list[Call]


class RecordingGuardrail:
    def __init__(self, replacement: Replacement | None) -> None:
        self.replacement = replacement
        self.calls: list[RecordedCall] = []

    async def apply_guardrail(
        self,
        inputs: GuardrailInputs,
        request_data: Record,
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> GuardrailInputs:
        del logging_obj
        self.calls.append(
            {
                "texts": list(inputs["texts"]),
                "model": inputs.get("model"),
                "input_type": input_type,
                "request_data": deepcopy(request_data),
            }
        )
        if self.replacement is None:
            return inputs
        return {"texts": self.replacement}


def result(guardrail: RecordingGuardrail, data: JsonValue) -> JsonValue:
    if not guardrail.calls:
        skipped: CallResult = {
            "called": False,
            "call_count": 0,
            "texts": None,
            "model": None,
            "input_type": None,
            "request_data": None,
            "data": data,
        }
        return JSON.validate_python(skipped)
    recorded = guardrail.calls[0]
    called: CallResult = {
        "called": True,
        "call_count": len(guardrail.calls),
        "texts": recorded["texts"],
        "model": recorded["model"],
        "input_type": recorded["input_type"],
        "request_data": recorded["request_data"],
        "data": data,
    }
    return JSON.validate_python(called)


def request_case(call: RequestCall) -> JsonValue:
    guardrail = RecordingGuardrail(call.get("replacement"))
    data = deepcopy(call["data"])
    asyncio.run(HANDLER.process_input_messages(data=data, guardrail_to_apply=guardrail))
    return result(guardrail, data)


def response_case(call: ResponseCall) -> JsonValue:
    guardrail = RecordingGuardrail(call.get("replacement"))
    response = deepcopy(call["response"])
    asyncio.run(
        HANDLER.process_output_response(
            response=response,
            guardrail_to_apply=guardrail,
            user_api_key_dict=call.get("user_api_key_dict"),
            request_data=call.get("request_data"),
        )
    )
    return result(guardrail, response)


def stream_case(call: StreamCall) -> JsonValue:
    guardrail = RecordingGuardrail(call.get("replacement"))
    frames = list(call["frames"])
    asyncio.run(
        HANDLER.process_output_streaming_response(
            responses_so_far=frames,
            guardrail_to_apply=guardrail,
            user_api_key_dict=call.get("user_api_key_dict"),
            request_data=call.get("request_data"),
        )
    )
    return result(guardrail, frames)


def chunk_value(chunk: SseChunk) -> str | bytes:
    if chunk["kind"] == "bytes":
        return bytes(chunk["value"])
    return chunk["value"]


def sse_case(call: SseCall) -> JsonValue:
    guardrail = RecordingGuardrail(None)
    chunks = [chunk_value(chunk) for chunk in call["chunks"]]
    asyncio.run(
        HANDLER.process_output_streaming_response(
            responses_so_far=chunks, guardrail_to_apply=guardrail
        )
    )
    if not guardrail.calls:
        empty: SseResult = {"text": None}
        return JSON.validate_python(empty)
    joined: SseResult = {"text": "".join(guardrail.calls[0]["texts"])}
    return JSON.validate_python(joined)


def resolve(call: Call) -> JsonValue:
    match call["function"]:
        case "request":
            return request_case(call)
        case "response":
            return response_case(call)
        case "stream":
            return stream_case(call)
        case _:
            return sse_case(call)


def run(input_value: Input) -> JsonValue:
    outcome: RunResult = {"calls": [resolve(call) for call in input_value["calls"]]}
    return JSON.validate_python(outcome)
