import asyncio
import json
import time
import urllib.parse
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.caching.dual_cache import DualCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.agent_365.agent_365 import Agent365Guardrail
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
FORM_CONTENT_TYPE: Final = "application/x-www-form-urlencoded"
GUARDRAIL_NAME: Final = "agent-365-guard"
FIXED_NOW: Final = 1_000_000.0


class ScriptedResponse(TypedDict):
    kind: Literal["reply"]
    status: int
    body: JsonValue
    raw: str | None


class ScriptedUnreachable(TypedDict):
    kind: Literal["unreachable"]


ScriptedReply: TypeAlias = ScriptedResponse | ScriptedUnreachable


class Invocation(TypedDict):
    call_type: str
    data: dict[str, JsonValue]


class Enforcement(TypedDict):
    unreachable_fallback: Literal["fail_closed", "fail_open"]
    key_alias: str | None
    replies: list[ScriptedReply]
    repeat_last_reply: bool
    invocations: list[Invocation]


class Captured(TypedDict):
    url: str
    authorization: str | None
    content_type: str | None
    body: JsonValue


class Bucket(TypedDict, total=False):
    standard_logging_guardrail_information: list[JsonValue]


class PreCallHook(Protocol):
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: dict[str, JsonValue],
        call_type: str,
    ) -> Exception | str | dict[str, JsonValue] | None: ...


BUCKETS: Final = TypeAdapter(Bucket)


class ScriptedReplies:
    def __init__(self, replies: list[ScriptedReply], *, repeat_last_reply: bool) -> None:
        self.replies = replies
        self.repeat_last_reply = repeat_last_reply
        self.requests: list[Captured] = []
        self.used = 0

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(capture_request(request))
        if self.used < len(self.replies):
            reply = self.replies[self.used]
            self.used += 1
        elif self.repeat_last_reply and self.replies:
            reply = self.replies[-1]
        else:
            raise ValueError("the scripted replies are exhausted")
        if reply["kind"] == "unreachable":
            raise httpx.ReadTimeout("the scripted Agent 365 endpoint did not answer")
        if reply["raw"] is not None:
            return httpx.Response(reply["status"], content=reply["raw"])
        return httpx.Response(reply["status"], json=reply["body"])


def capture_request(request: httpx.Request) -> Captured:
    content_type = request.headers.get("content-type")
    if content_type == FORM_CONTENT_TYPE:
        parsed = urllib.parse.parse_qs(request.content.decode(), keep_blank_values=True)
        fields = {name: values[0] for name, values in parsed.items()}
        body: JsonValue = JSON.validate_python(fields)
    else:
        body = JSON.validate_json(request.content)
    return {
        "url": str(request.url),
        "authorization": request.headers.get("authorization"),
        "content_type": content_type,
        "body": body,
    }


def records_of(data: dict[str, JsonValue]) -> list[JsonValue]:
    metadata = data.get("metadata")
    if metadata is None:
        return []
    bucket = BUCKETS.validate_json(json.dumps(metadata))
    if "standard_logging_guardrail_information" not in bucket:
        return []
    return bucket["standard_logging_guardrail_information"]


async def call_hook(
    hook: PreCallHook, auth: UserAPIKeyAuth, invocation: Invocation
) -> JsonValue:
    try:
        await hook.async_pre_call_hook(
            user_api_key_dict=auth,
            cache=DualCache(),
            data=invocation["data"],
            call_type=invocation["call_type"],
        )
    except HTTPException as error:
        detail = JSON.validate_python(error.detail)
        return JSON.validate_python(
            {"kind": "rejected", "status_code": error.status_code, "detail": detail}
        )
    return JSON.validate_python({"kind": "allowed"})


async def run_invocations(
    hook: PreCallHook, auth: UserAPIKeyAuth, args: Enforcement
) -> tuple[list[JsonValue], list[JsonValue]]:
    outcomes: list[JsonValue] = []
    records: list[JsonValue] = []
    for invocation in args["invocations"]:
        outcomes.append(await call_hook(hook, auth, invocation))
        records.append(JSON.validate_python(records_of(invocation["data"])))
    return outcomes, records


def run(input_value: Enforcement) -> JsonValue:
    replies = ScriptedReplies(
        input_value["replies"], repeat_last_reply=input_value["repeat_last_reply"]
    )
    guardrail = Agent365Guardrail(
        GUARDRAIL_NAME,
        "tenant-abc",
        "client-xyz",
        "secret-123",
        unreachable_fallback=input_value["unreachable_fallback"],
        async_handler=AsyncHTTPHandler(transport=httpx.MockTransport(replies)),
        event_hook=GuardrailEventHooks.pre_mcp_call,
        default_on=True,
    )
    auth = UserAPIKeyAuth(key_alias=input_value["key_alias"])
    wall_clock = time.time
    perf_clock = time.perf_counter
    time.time = partial(float, FIXED_NOW)
    time.perf_counter = partial(float, FIXED_NOW)
    try:
        outcomes, records = asyncio.run(run_invocations(guardrail, auth, input_value))
    finally:
        time.time = wall_clock
        time.perf_counter = perf_clock
    return JSON.validate_python(
        {"outcomes": outcomes, "records": records, "requests": replies.requests}
    )
