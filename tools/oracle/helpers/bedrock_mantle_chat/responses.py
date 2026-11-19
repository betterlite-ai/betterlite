import httpx
from pydantic import JsonValue

from .inputs import NativeSseCall, Object, ResponseCall
from .views import CHAT, CHAT_PROBE, MESSAGES, LoggingStub


def native_sse(call: NativeSseCall) -> JsonValue:
    # the case carries each native event's JSON; the iterator reads the SSE data: framing
    events = [f"data: {event}\n\n" for event in call["events"]]
    iterator = CHAT.get_model_response_iterator(
        iter(events), sync_stream=False, json_mode=None
    )
    chunks: list[JsonValue] = []
    for chunk in iterator:
        choice = chunk.choices[0]
        content = choice.delta.content
        finish_reason = choice.finish_reason
        view: Object = {
            "content": "" if content is None else content,
            "finish_reason": None if finish_reason == "" else finish_reason,
        }
        chunks.append(view)
    outcome: Object = {
        "has_custom_stream_wrapper": CHAT_PROBE.has_custom_stream_wrapper,
        "chunks": chunks,
    }
    return outcome


def messages_response(call: ResponseCall) -> JsonValue:
    response = MESSAGES.transform_anthropic_messages_response(
        "claude",
        httpx.Response(call["status"], content=call["body"].encode("utf-8")),
        LoggingStub(),
    )
    outcome: Object = {"response": response}
    return outcome
