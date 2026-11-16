import ssl as ssl_module
from typing import TypeAlias

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.decisions.transformation import OpenAIDecisionRequestBody
from litellm.types.llms.openai import OpenAICreateThreadParamsMessage
from litellm.types.realtime import RealtimeQueryParams
from openai.types.beta import AssistantToolParam
from pydantic import JsonValue, TypeAdapter
from typing_extensions import Literal, NotRequired, Protocol, TypedDict, Unpack

ORG: str = "org-fixture"
FAILURE: str = "stop"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
DECISION: TypeAdapter[OpenAIDecisionRequestBody] = TypeAdapter(OpenAIDecisionRequestBody)
SslValue: TypeAlias = bool | str | ssl_module.SSLContext | None

Params = TypedDict(
    "Params", {"order": str | None, "limit": int | None, "before": str | None, "after": str | None}
)
Canned = TypedDict(
    "Canned",
    {"status": int, "body": NotRequired[JsonValue], "text": NotRequired[str], "content_type": NotRequired[str]},
)
RunBody = TypedDict(
    "RunBody",
    {
        "thread_id": str,
        "assistant_id": str,
        "additional_instructions": str | None,
        "instructions": str | None,
        "metadata": dict[str, str] | None,
        "model": str | None,
        "tools": list[AssistantToolParam] | None,
    },
)
ThreadBody = TypedDict(
    "ThreadBody",
    {"messages": list[OpenAICreateThreadParamsMessage] | None, "metadata": dict[str, str] | None},
)
THREAD_CREATE: TypeAdapter[ThreadBody] = TypeAdapter(ThreadBody)
Input = TypedDict(
    "Input",
    {
        "kind": Literal["assistants", "threads", "runs", "decisions", "realtime"],
        "operation": Literal[
            "async_list", "create", "delete", "retrieve", "add_message",
            "list_messages", "buffered", "stream", "request", "response", "connect",
        ],
        "api_base": str | None,
        "api_key": str | None,
        "params": Params,
        "aget_assistants": bool | None,
        "arun_thread": bool,
        "data": dict[str, JsonValue],
        "request": RunBody,
        "decision": dict[str, JsonValue],
        "payload": str,
        "model": str,
        "replies": list[Canned],
        "assistant_id": str,
        "thread_id": str,
        "query_params": RealtimeQueryParams | None,
        "openai_beta": bool,
        "scope_headers": list[tuple[str, str]],
        "close_error": str | None,
        "max_message_size_bytes": int,
    },
)


class ClientSocket(Protocol):
    async def send_text(self, data: str) -> None: ...

    async def receive_text(self) -> str: ...

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


RealtimeCall = TypedDict(
    "RealtimeCall",
    {
        "model": str,
        "websocket": ClientSocket,
        "logging_obj": Logging,
        "api_base": str | None,
        "api_key": str | None,
        "timeout": float | None,
        "query_params": RealtimeQueryParams | None,
        "litellm_metadata": dict[str, JsonValue] | None,
    },
)


class RealtimeRunner(Protocol):
    async def async_realtime(self, **call: Unpack[RealtimeCall]) -> None: ...
