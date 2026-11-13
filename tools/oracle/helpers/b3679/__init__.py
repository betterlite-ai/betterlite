from typing import Literal, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail.unified_guardrail import (
    resolve_endpoint_translation,
)
from pydantic import JsonValue as Json


class EntryCall(TypedDict):
    call_type: str
    translation: str
    chat_completions: bool


class EndpointResolutionCall(TypedDict):
    function: Literal["endpoint_resolution"]
    request_route: str | None
    first_item: Json | None
    entries: list[EntryCall]


def run(call: EndpointResolutionCall) -> Json:
    auth = UserAPIKeyAuth(request_route=call["request_route"])
    resolved = resolve_endpoint_translation(auth, call["first_item"])
    if resolved is None:
        return None
    call_type, handler = resolved
    return {"call_type": call_type, "translation": type(handler).__name__}
