import os
from collections.abc import Awaitable
from typing import TypedDict

import httpx
import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.types.agents import AgentResponse
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

KEPT_HEADERS = ("authorization", "content-type", "x-api-key", "x-card")


class AgentCase(TypedDict):
    agent_name: str
    agent_card_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]


class GlobalsCase(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None


class UpstreamCase(TypedDict):
    status: int
    body: str


class DispatchCase(TypedDict):
    platform: str
    model: str
    messages: list[dict[str, JsonValue]]
    stream: bool | None
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None
    optionals: dict[str, JsonValue]
    globals: GlobalsCase
    environment: dict[str, str]
    registry: list[AgentCase]
    upstream: UpstreamCase


class Responder:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.url = ""
        self.headers: dict[str, str] = {}
        self.body_text = ""
        self.seen = False

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen = True
        self.url = str(request.url)
        self.headers = dict(request.headers)
        self.body_text = request.content.decode()
        return httpx.Response(self.status, content=self.body.encode())


def register(agent: AgentCase) -> None:
    fields: dict[str, JsonValue] = {
        "agent_id": agent["agent_name"],
        "agent_name": agent["agent_name"],
        "agent_card_params": agent["agent_card_params"],
        "litellm_params": agent["litellm_params"],
    }
    global_agent_registry.register_agent(TypeAdapter(AgentResponse).validate_python(fields))


def kept_headers(headers: dict[str, str]) -> dict[str, JsonValue]:
    kept: dict[str, JsonValue] = {}
    for name, value in headers.items():
        if name in KEPT_HEADERS:
            kept[name] = value
    return kept


def request_view(responder: Responder) -> JsonValue | None:
    if not responder.seen:
        return None
    return {
        "url": responder.url,
        "headers": kept_headers(responder.headers),
        "body": TypeAdapter(JsonValue).validate_json(responder.body_text),
    }


def reply_view(response: ModelResponse) -> JsonValue:
    choice = response.choices[0]
    return {
        "model": response.model,
        "content": choice.message.content,
        "role": choice.message.role,
        "finish_reason": choice.finish_reason,
    }


def error_view(responder: Responder, status: int | None) -> JsonValue:
    return {
        "request": request_view(responder),
        "reply": None,
        "error": {"status": status},
    }


def dispatch_case(case: DispatchCase) -> JsonValue:
    os.environ.update(case["environment"])
    for agent in case["registry"]:
        register(agent)
    responder = Responder(case["upstream"]["status"], case["upstream"]["body"])
    transport = httpx.MockTransport(responder.handle)
    client = HTTPHandler(client=httpx.Client(transport=transport))
    try:
        # litellm.completion's decorated return is Any to mypy, so its type is named here.
        response: (
            ModelResponse | CustomStreamWrapper | Awaitable[ModelResponse | CustomStreamWrapper]
        ) = litellm.completion(
            model=f"{case['platform']}/{case['model']}",
            messages=case["messages"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            headers=case["headers"],
            stream=case["stream"],
            client=client,
            **case["optionals"],
        )
    except openai.APIStatusError as error:
        return error_view(responder, error.status_code)
    except openai.APIConnectionError:
        return error_view(responder, None)
    # a streaming call returns a stream rather than a reply, and the stream's chunks are not part of this case
    if case["stream"] is True:
        return {"request": request_view(responder), "reply": None, "error": None}
    reply = TypeAdapter(ModelResponse).validate_python(response)
    return {
        "request": request_view(responder),
        "reply": reply_view(reply),
        "error": None,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(DispatchCase).validate_python(input_value)
    return dispatch_case(case)
