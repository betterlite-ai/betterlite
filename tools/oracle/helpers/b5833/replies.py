from typing import TypedDict

import httpx
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .views import CHAT, logging_object
from .wire import MODEL, Call, Object


class ServerUse(TypedDict, total=False):
    web_search_requests: int | None
    browser_open_requests: int | None


class ReplyUsage(TypedDict, total=False):
    server_tool_use: ServerUse


class ReplyRecord(TypedDict, total=False):
    service_tier: str | None
    usage: ReplyUsage


REPLY_RECORD: TypeAdapter[ReplyRecord] = TypeAdapter(ReplyRecord)


def reply_value(call: Call) -> JsonValue:
    body = call["reply"]
    response: ModelResponse = CHAT.transform_response(
        MODEL,
        httpx.Response(200, json=body),
        ModelResponse(),
        logging_object(),
        {},
        [],
        {},
        {},
        None,
        json_mode=call.get("json_mode"),
    )
    record = REPLY_RECORD.validate_json(response.model_dump_json())
    usage = record.get("usage")
    server: ServerUse | None = None
    if usage is not None:
        server = usage.get("server_tool_use")
    searches: int | None = None
    opens: int | None = None
    if server is not None:
        searches = server.get("web_search_requests")
        opens = server.get("browser_open_requests")
    value: Object = {
        "service_tier": record.get("service_tier"),
        "web_search_requests": searches,
        "browser_open_requests": opens,
    }
    return value
