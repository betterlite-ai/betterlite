import asyncio
from collections.abc import Callable
from typing import Literal, NotRequired, Protocol, Required, TypedDict, TypeVar

import httpx
from litellm.exceptions import GuardrailRaisedException
from litellm.proxy.guardrails.guardrail_hooks.promptguard.promptguard import (
    PromptGuardGuardrail,
)
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter
from pydantic_core import to_json

JSON = TypeAdapter(JsonValue)
Mode = Literal["request", "response"]


class Answer(TypedDict):
    status: int
    body: dict[str, JsonValue]


class Input(TypedDict, total=False):
    api_key: Required[str]
    block_on_error: bool
    inputs: Required[JsonValue]
    input_type: Required[Mode]
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


Data_contra = TypeVar("Data_contra", contravariant=True)


class Reply(TypedDict, total=False):
    texts: list[str]


class Guard(Protocol[Data_contra]):
    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: Data_contra,
        input_type: Mode,
    ) -> Reply: ...


async def scan(
    guard: Guard[Data_contra], inputs: JsonValue, empty: Callable[[], Data_contra], mode: Mode
) -> Reply:
    return await guard.apply_guardrail(
        TypeAdapter(GenericGuardrailAPIInputs).validate_python(inputs), empty(), mode  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536-L4547
    )


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
            scan(guardrail, input_value["inputs"], dict, input_value["input_type"])
        )
    except GuardrailRaisedException as error:
        return JSON.validate_python(raised_view(captured, error))
    output: Output = {
        "request": request_view(captured),
        "inputs": JSON.validate_json(to_json(result)),
    }
    return JSON.validate_python(output)
