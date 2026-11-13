import asyncio
from typing import Literal, NotRequired, Required, TypedDict

import httpx
from litellm.exceptions import GuardrailRaisedException
from litellm.proxy.guardrails.guardrail_hooks.promptguard.promptguard import (
    PromptGuardGuardrail,
)
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter

JSON = TypeAdapter(JsonValue)


class Answer(TypedDict):
    status: int
    body: dict[str, JsonValue]


class Input(TypedDict, total=False):
    api_key: Required[str]
    block_on_error: bool
    inputs: Required[GenericGuardrailAPIInputs]
    input_type: Required[Literal["request", "response"]]
    answer: Required[Answer]


class Captured:
    def __init__(self) -> None:
        self.method: str | None = None
        self.path: str | None = None
        self.body: JsonValue = None
        self.content_type: str | None = None
        self.api_key_present: bool = False


class Transport:
    def __init__(self, answer: Answer, captured: Captured) -> None:
        self.answer = answer
        self.captured = captured

    def __call__(self, request: httpx.Request) -> httpx.Response:
        captured = self.captured
        captured.method = request.method
        captured.path = request.url.path
        captured.body = JSON.validate_json(request.content)
        captured.content_type = request.headers.get("Content-Type")
        captured.api_key_present = "X-API-Key" in request.headers
        return httpx.Response(self.answer["status"], json=self.answer["body"])


class RequestView(TypedDict):
    method: str | None
    path: str | None
    body: JsonValue
    content_type: str | None
    api_key_present: bool


class RaisedView(TypedDict):
    status: int
    blocked_content: bool


class Output(TypedDict):
    request: RequestView
    inputs: NotRequired[JsonValue]
    raised: NotRequired[RaisedView]


def request_view(captured: Captured) -> RequestView:
    return {
        "method": captured.method,
        "path": captured.path,
        "body": captured.body,
        "content_type": captured.content_type,
        "api_key_present": captured.api_key_present,
    }


def raised_view(captured: Captured, error: GuardrailRaisedException) -> Output:
    raised: RaisedView = {
        "status": error.status_code,
        "blocked_content": error.blocked_content,
    }
    return {"request": request_view(captured), "raised": raised}


def run(input_value: Input) -> JsonValue:
    guardrail = PromptGuardGuardrail(
        api_key=input_value["api_key"],
        block_on_error=input_value.get("block_on_error"),
    )
    captured = Captured()
    transport = Transport(input_value["answer"], captured)
    guardrail.async_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(transport)
    )
    try:
        result = asyncio.run(
            guardrail.apply_guardrail(
                inputs=input_value["inputs"],
                request_data={},
                input_type=input_value["input_type"],
            )
        )
    except GuardrailRaisedException as error:
        return JSON.validate_python(raised_view(captured, error))
    return JSON.validate_python({"request": request_view(captured), "inputs": result})
