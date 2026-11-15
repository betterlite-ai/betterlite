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


class RecordedCall(TypedDict):
    texts: list[str]
    request_data: Record


class Outcome(TypedDict):
    texts: list[str] | None
    request_data: Record | None
    data: JsonValue


class SseResult(TypedDict):
    text: str | None


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
        del input_type, logging_obj
        recorded: RecordedCall = {
            "texts": list(inputs["texts"]),
            "request_data": deepcopy(request_data),
        }
        self.calls.append(recorded)
        return inputs if self.replacement is None else {"texts": self.replacement}


class GoogleHandler(Protocol):
    async def process_input_messages(
        self,
        data: Record,
        guardrail_to_apply: RecordingGuardrail,
        litellm_logging_obj: Logging | None = None,
    ) -> None: ...

    async def process_output_response(
        self,
        response: Record,
        guardrail_to_apply: RecordingGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: Record | None = None,
        request_data: Record | None = None,
    ) -> None: ...

    async def process_output_streaming_response(  # ast-grep-ignore: no-long-names
        self,
        responses_so_far: Sequence[JsonValue | bytes],
        guardrail_to_apply: RecordingGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: Record | None = None,
        request_data: Record | None = None,
    ) -> None: ...


# LiteLLM declares dict and UserAPIKeyAuth, while its code accepts plain dicts
HANDLER: GoogleHandler = cast(GoogleHandler, GoogleGenAIGenerateContentHandler())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/google_genai/guardrail_translation/handler.py#L158-L255


class Guarded(TypedDict):
    replacement: NotRequired[Replacement]
    request_data: NotRequired[Record]
    user_api_key_dict: NotRequired[Record]


class RequestCall(Guarded):
    function: Literal["request"]
    data: Record


class ResponseCall(Guarded):
    function: Literal["response"]
    response: Record


class StreamCall(Guarded):
    function: Literal["stream"]
    frames: list[JsonValue]


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


def result(guardrail: RecordingGuardrail, data: JsonValue) -> JsonValue:
    recorded = guardrail.calls[0] if guardrail.calls else None
    outcome: Outcome = {
        "texts": recorded["texts"] if recorded is not None else None,
        "request_data": recorded["request_data"] if recorded is not None else None,
        "data": data,
    }
    return JSON.validate_python(outcome)


def guarded_case(call: RequestCall | ResponseCall | StreamCall) -> JsonValue:
    guardrail = RecordingGuardrail(call.get("replacement"))
    match call["function"]:
        case "request":
            data = deepcopy(call["data"])
            asyncio.run(HANDLER.process_input_messages(data=data, guardrail_to_apply=guardrail))
            return result(guardrail, data)
        case "response":
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
        case _:
            frames = deepcopy(call["frames"])
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
    if call["function"] == "sse":
        return sse_case(call)
    return guarded_case(call)


def run(input_value: Input) -> JsonValue:
    outcome: dict[str, list[JsonValue]] = {
        "calls": [resolve(call) for call in input_value["calls"]]
    }
    return JSON.validate_python(outcome)
