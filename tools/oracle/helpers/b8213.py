import asyncio
import json
from typing import Literal, NotRequired, TypedDict

import httpx
from litellm.proxy.guardrails.guardrail_hooks.qualifire.qualifire import (
    QualifireGuardrail,
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Answer(TypedDict):
    status: int
    body: JsonValue


class Settings(TypedDict, total=False):
    api_key: str
    api_base: str
    evaluation_id: str
    prompt_injections: bool
    hallucinations_check: bool
    grounding_check: bool
    pii_check: bool
    content_moderation_check: bool
    tool_selection_quality_check: bool
    assertions: list[str]
    on_flagged: str
    timeout: float


class ChoiceMessage(TypedDict):
    content: str


class Choice(TypedDict):
    message: ChoiceMessage


class Response(TypedDict):
    choices: list[Choice]


class Inputs(TypedDict, total=False):
    texts: list[str]
    structured_messages: list[dict[str, JsonValue]]
    model: str


class Call(TypedDict):
    settings: Settings
    messages: list[dict[str, JsonValue]]
    answer: Answer
    tools: NotRequired[list[dict[str, JsonValue]]]
    response: NotRequired[Response]


class Captured(TypedDict):
    path: str
    body: JsonValue
    x_qualifire_api_key: str
    content_type: str


class Result(TypedDict):
    request: Captured | None
    status: int | None
    detail: JsonValue


class Reply:
    def __init__(self, answer: Answer, captured: list[Captured]) -> None:
        self.answer = answer
        self.captured = captured

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.captured.append(
            {
                "path": request.url.path,
                "body": JSON.validate_json(request.content),
                "x_qualifire_api_key": request.headers["x-qualifire-api-key"],
                "content_type": request.headers["content-type"],
            }
        )
        body = json.dumps(self.answer["body"]).encode()
        return httpx.Response(status_code=self.answer["status"], content=body)


def build(call: Call) -> QualifireGuardrail:
    settings = call["settings"]
    return QualifireGuardrail(
        api_key=settings.get("api_key"),
        api_base=settings.get("api_base"),
        evaluation_id=settings.get("evaluation_id"),
        prompt_injections=settings.get("prompt_injections"),
        hallucinations_check=settings.get("hallucinations_check"),
        grounding_check=settings.get("grounding_check"),
        pii_check=settings.get("pii_check"),
        content_moderation_check=settings.get("content_moderation_check"),
        tool_selection_quality_check=settings.get("tool_selection_quality_check"),
        assertions=settings.get("assertions"),
        on_flagged=settings.get("on_flagged"),
        timeout=settings.get("timeout"),
        guardrail_name="qualifire",
    )


def request_data(call: Call) -> dict[str, JsonValue]:
    data: dict[str, JsonValue] = {"messages": list(call["messages"])}
    tools = call.get("tools")
    if tools is not None:
        data["tools"] = JSON.validate_python(tools)
    if "response" not in call:
        data["model"] = "gpt-4o"
    return data


def guardrail_inputs(call: Call) -> Inputs:
    response = call.get("response")
    if response is not None:
        return {"texts": [choice["message"]["content"] for choice in response["choices"]]}
    return {"structured_messages": list(call["messages"]), "model": "gpt-4o"}


def outcome(
    guardrail: QualifireGuardrail,
    call: Call,
    captured: list[Captured],
) -> Result:
    status: int | None = None
    detail: JsonValue = None
    kind: Literal["request", "response"] = "response" if "response" in call else "request"
    try:
        asyncio.run(guardrail.apply_guardrail(guardrail_inputs(call), request_data(call), kind))  # type: ignore[misc]  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/qualifire/qualifire.py#L423
    except HTTPException as error:
        status = error.status_code
        detail = JSON.validate_python(error.detail)
    except httpx.HTTPStatusError as error:
        status = error.response.status_code
    return {
        "request": captured[0] if captured else None,
        "status": status,
        "detail": detail,
    }


def run(call: Call) -> JsonValue:
    captured: list[Captured] = []
    guardrail = build(call)
    transport = httpx.MockTransport(Reply(call["answer"], captured))
    guardrail.async_handler.client = httpx.AsyncClient(transport=transport)
    return JSON.validate_python(outcome(guardrail, call, captured))
