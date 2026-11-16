import asyncio
import datetime as dt
import time
import urllib.parse
import uuid
from functools import partial
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.agent_365.agent_365 import (
    Agent365Guardrail,
    Agent365MalformedResponseError,
    Agent365ThrottledError,
    Agent365TokenExchangeError,
    entra_assertion,
)
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import BaseModel, Field, JsonValue, TypeAdapter, ValidationError

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
FIELDS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
UUID_MARKER = "<uuid4>"
UUID_VERSION = 4
FIXED_START = dt.datetime(1970, 1, 12, 13, 46, 40, tzinfo=dt.UTC)
FIXED_NOW = FIXED_START.timestamp()


class ScriptedReply(TypedDict):
    status: int
    body: JsonValue
    raw: str | None


class TokenRequest(TypedDict):
    kind: Literal["token"]
    assertion: str


class RangeRequest(TypedDict):
    kind: Literal["token_range"]
    prefix: str
    count: int


class PostRequest(TypedDict):
    kind: Literal["post"]
    url: str
    headers: dict[str, str]
    data: dict[str, str]


TokenStep: TypeAlias = TokenRequest | RangeRequest | PostRequest


class TokenExchange(TypedDict):
    function: Literal["agent_365_token_exchange"]
    replies: list[ScriptedReply]
    repeat_last_reply: bool
    steps: list[TokenStep]


class AssertionCheck(TypedDict):
    function: Literal["agent_365_assertion"]
    values: list[str]


class RequestPayload(TypedDict):
    litellm_logging_obj: Logging
    litellm_call_id: str
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    mcp_tool_name: str | None
    mcp_server_name: str | None
    mcp_arguments: dict[str, JsonValue] | None


class PayloadAssembly(TypedDict):
    function: Literal["agent_365_payload_assembly"]
    key_alias: str | None
    mcp_session_id: str | None
    litellm_call_id: str
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    mcp_tool_name: str | None
    mcp_server_name: str | None
    mcp_arguments: dict[str, JsonValue] | None


Input: TypeAlias = TokenExchange | AssertionCheck | PayloadAssembly


class EvaluatePayload(BaseModel):
    tool: dict[str, str]
    server_name: str = Field(alias="serverName")
    conversation_id: str = Field(alias="conversationId")
    arguments: dict[str, JsonValue] | None = None
    agent_id: str | None = Field(default=None, alias="agentId")


class LoggingDetails(Protocol):
    model_call_details: dict[str, JsonValue]


class ScriptedReplies:
    def __init__(self, replies: list[ScriptedReply], *, repeat_last_reply: bool) -> None:
        self.replies = replies
        self.repeat_last_reply = repeat_last_reply
        self.requests: list[dict[str, JsonValue]] = []
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
        if reply["raw"] is not None:
            return httpx.Response(reply["status"], content=reply["raw"])
        return httpx.Response(reply["status"], json=reply["body"])


class _ProbeMeta(type):
    pass


class Probe(Agent365Guardrail, metaclass=_ProbeMeta):
    # the metaclass keeps mypy's __init_subclass__ check away from LiteLLM's untyped kwargs
    async def fetch_token(self, assertion: str) -> str:
        return await self._get_obo_token(assertion)

    async def post_allowing(
        self, url: str, headers: dict[str, str], data: dict[str, str]
    ) -> httpx.Response:
        return await self._post_allowing_error_status(url=url, headers=headers, data=data)

    def cache_entries(self) -> list[list[JsonValue]]:
        return [
            [key, token, int(expires_at - FIXED_NOW)]
            for key, (token, expires_at) in self._obo_token_cache.items()
        ]

    def build_payload(self, data: RequestPayload, auth: UserAPIKeyAuth) -> JsonValue:
        payload = self._build_evaluate_payload(data, auth)
        return JSON.validate_python(payload)


def capture_request(request: httpx.Request) -> dict[str, JsonValue]:
    body = request.content.decode()
    parsed = urllib.parse.parse_qs(body, keep_blank_values=True)
    fields: dict[str, str] = {name: values[0] for name, values in parsed.items()}
    form = JSON.validate_python(fields)
    return {"url": str(request.url), "content_type": request.headers["content-type"], "form": form}


def make_guardrail() -> Probe:
    return Probe(
        "agent-365-guard", "tenant-abc", "client-xyz", "secret-123",
        event_hook=GuardrailEventHooks.pre_mcp_call, default_on=True,
    )


def exchange_outcome(error: Agent365TokenExchangeError) -> dict[str, JsonValue]:
    codes: list[int] = list(error.aadsts_codes)
    return {
        "kind": "token", "error": "exchange", "status_code": error.status_code,
        "error_code": error.error_code, "description": error.description,
        "aadsts_codes": JSON.validate_python(codes),
        "gateway_owned": error.gateway_owned,
    }


def malformed_variant(handler: ScriptedReplies) -> str:
    reply = handler.replies[min(handler.used, len(handler.replies)) - 1]
    try:
        body = reply["body"] if reply["raw"] is None else JSON.validate_json(reply["raw"])
    except ValidationError:
        return "non_json_body"
    try:
        fields = FIELDS.validate_python(body)
    except ValidationError:
        return "non_object_body"
    if "access_token" not in fields:
        return "missing_access_token"
    return "bad_access_token"


async def token_outcome(
    guardrail: Probe, assertion: str, handler: ScriptedReplies
) -> dict[str, JsonValue]:
    try:
        return {"kind": "token", "access_token": await guardrail.fetch_token(assertion)}
    except Agent365TokenExchangeError as error:
        return exchange_outcome(error)
    except Agent365ThrottledError as error:
        return {"kind": "token", "error": "throttled", "status_code": error.status_code}
    except Agent365MalformedResponseError:
        return {"kind": "token", "error": "malformed", "variant": malformed_variant(handler)}
    except httpx.HTTPStatusError as error:
        return {"kind": "token", "error": "http_status", "status_code": error.response.status_code}


async def range_outcome(guardrail: Probe, prefix: str, count: int) -> dict[str, JsonValue]:
    token = ""
    for index in range(count):
        token = await guardrail.fetch_token(f"{prefix}{index}")
    return {"kind": "token_range", "access_token": token}


async def post_outcome(guardrail: Probe, step: PostRequest) -> dict[str, JsonValue]:
    response = await guardrail.post_allowing(step["url"], step["headers"], step["data"])
    body = JSON.validate_json(response.content)
    return {"kind": "post", "status": response.status_code, "body": body}


async def run_step(
    guardrail: Probe, step: TokenStep, handler: ScriptedReplies
) -> dict[str, JsonValue]:
    if step["kind"] == "token":
        return await token_outcome(guardrail, step["assertion"], handler)
    if step["kind"] == "token_range":
        return await range_outcome(guardrail, step["prefix"], step["count"])
    return await post_outcome(guardrail, step)


async def run_exchange(
    guardrail: Probe, steps: list[TokenStep], handler: ScriptedReplies
) -> list[JsonValue]:
    outcomes: list[JsonValue] = [await run_step(guardrail, step, handler) for step in steps]
    await guardrail.async_handler.client.aclose()
    return outcomes


def exchange_result(args: TokenExchange) -> JsonValue:
    handler = ScriptedReplies(args["replies"], repeat_last_reply=args["repeat_last_reply"])
    guardrail = make_guardrail()
    guardrail.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    # the Rust helper fixes its clock at the same second, so cache lifetimes compare exactly
    wall_clock = time.time
    time.time = partial(float, FIXED_NOW)
    try:
        outcomes = asyncio.run(run_exchange(guardrail, args["steps"], handler))
    finally:
        time.time = wall_clock
    return {
        "requests": JSON.validate_python(handler.requests),
        "steps": JSON.validate_python(outcomes),
        "cache": JSON.validate_python(guardrail.cache_entries()),
    }


def assertion_result(args: AssertionCheck) -> JsonValue:
    accepted = [value for value in args["values"] if entra_assertion(value) is not None]
    return {"accepted": JSON.validate_python(accepted)}


def payload_request(args: PayloadAssembly) -> RequestPayload:
    logging_obj = Logging(
        model="mcp", messages=None, stream=False, call_type="call_mcp_tool",
        start_time=FIXED_START, litellm_call_id=args["litellm_call_id"], function_id="fn-1",
    )
    session = args["mcp_session_id"]
    if session is not None:
        details: LoggingDetails = logging_obj
        details.model_call_details["mcp_tool_call_metadata"] = {"mcp_session_id": session}
    # LiteLLM reads each field with data.get, so a None value reads the same as an absent key
    return {
        "litellm_call_id": args["litellm_call_id"], "litellm_logging_obj": logging_obj,
        "metadata": args["metadata"], "litellm_metadata": args["litellm_metadata"],
        "mcp_tool_name": args["mcp_tool_name"], "mcp_server_name": args["mcp_server_name"],
        "mcp_arguments": args["mcp_arguments"],
    }


def payload_result(args: PayloadAssembly) -> JsonValue:
    request_data = payload_request(args)
    auth = UserAPIKeyAuth(key_alias=args["key_alias"])
    payload = EvaluatePayload.model_validate(make_guardrail().build_payload(request_data, auth))
    try:
        generated = uuid.UUID(payload.conversation_id).version == UUID_VERSION
    except ValueError:
        generated = False
    if generated:
        payload.conversation_id = UUID_MARKER
    return {
        "payload": JSON.validate_json(payload.model_dump_json(by_alias=True, exclude_none=True)),
        "conversation_id_is_uuid4": generated,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "agent_365_token_exchange":
        return exchange_result(input_value)
    if input_value["function"] == "agent_365_assertion":
        return assertion_result(input_value)
    return payload_result(input_value)
