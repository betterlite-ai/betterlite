import asyncio
import json
from typing import NotRequired, TypedDict

import httpx
from fastapi import HTTPException
from litellm.llms.openai.chat.guardrail_translation.handler import (
    OpenAIChatCompletionsHandler,
)
from litellm.proxy.guardrails.guardrail_hooks.qualifire.qualifire import (
    QualifireGuardrail,
)
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

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


class Call(TypedDict):
    settings: Settings
    messages: list[dict[str, JsonValue]]
    answer: Answer
    tools: NotRequired[list[dict[str, JsonValue]]]
    response: NotRequired[dict[str, JsonValue]]


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


def request_data(call: Call, *, request: bool) -> dict[str, JsonValue]:
    data: dict[str, JsonValue] = {"messages": list(call["messages"])}
    if request:
        data["model"] = "gpt-4o"
    tools = call.get("tools")
    if tools is not None:
        data["tools"] = JSON.validate_python(tools)
    return data


def fire_request(
    handler: OpenAIChatCompletionsHandler,
    guardrail: QualifireGuardrail,
    data: dict[str, JsonValue],
) -> None:
    asyncio.run(handler.process_input_messages(data, guardrail))  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/guardrail_translation/handler.py#L103


def fire_response(
    handler: OpenAIChatCompletionsHandler,
    guardrail: QualifireGuardrail,
    response: dict[str, JsonValue],
    data: dict[str, JsonValue],
) -> None:
    answer = ModelResponse.model_validate_json(json.dumps(response))
    asyncio.run(handler.process_output_response(answer, guardrail, None, None, data))  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/guardrail_translation/handler.py#L383


def outcome(
    handler: OpenAIChatCompletionsHandler,
    guardrail: QualifireGuardrail,
    call: Call,
    captured: list[Captured],
) -> Result:
    status: int | None = None
    detail: JsonValue = None
    response = call.get("response")
    try:
        if response is not None:
            fire_response(handler, guardrail, response, request_data(call, request=False))
        else:
            fire_request(handler, guardrail, request_data(call, request=True))
    except HTTPException as error:
        status = error.status_code
        detail = error.detail  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/qualifire/qualifire.py#L409
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
    handler = OpenAIChatCompletionsHandler()
    return JSON.validate_python(outcome(handler, guardrail, call, captured))
