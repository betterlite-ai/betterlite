import asyncio
import os
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.aporia_ai.aporia_ai import (  # pyright: ignore[reportMissingTypeStubs]
    AporiaGuardrail,
)
from litellm.proxy.guardrails.guardrail_hooks.guardrails_ai.guardrails_ai import (  # pyright: ignore[reportMissingTypeStubs]
    GuardrailsAI,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def mock_transport(answer: Json, captured: dict[str, Json]) -> httpx.MockTransport:
    def handle(request: httpx.Request) -> httpx.Response:
        captured["path"] = request.url.path
        captured["request"] = cast(Json, httpx.Response(200, content=request.content).json())
        match answer:
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    return httpx.MockTransport(handle)


def aporia_moderation(fields: dict[str, Json]) -> Json:
    aporia = AporiaGuardrail(api_key=cast(str, fields["api_key"]), api_base="https://aporia.test")
    captured: dict[str, Json] = {}
    aporia.async_handler.client = httpx.AsyncClient(
        transport=mock_transport(fields.get("answer"), captured)
    )
    data: dict[str, Json] = {}
    if fields.get("messages") is not None:
        data["messages"] = fields["messages"]
    with patch.object(aporia, "should_run_guardrail", return_value=True):
        asyncio.run(
            aporia.async_moderation_hook(  # pyright: ignore[reportUnknownMemberType]
                data=cast("dict[str, object]", data),
                user_api_key_dict=UserAPIKeyAuth(),
                call_type="completion",
            )
        )
    return {"path": captured.get("path"), "request": captured.get("request")}


def aporia_post_call(fields: dict[str, Json]) -> Json:
    aporia = AporiaGuardrail(api_key=cast(str, fields["api_key"]), api_base="https://aporia.test")
    captured: dict[str, Json] = {}
    aporia.async_handler.client = httpx.AsyncClient(
        transport=mock_transport(fields.get("answer"), captured)
    )
    data: dict[str, Json] = {"messages": fields["messages"]}
    response = litellm.ModelResponse(choices=[{"message": {"role": "assistant", "content": ""}}])
    convert_target = (
        "litellm.proxy.guardrails.guardrail_hooks.aporia_ai.aporia_ai"
        ".convert_litellm_response_object_to_str"
    )
    with (
        patch.object(aporia, "should_run_guardrail", return_value=True),
        patch(convert_target, return_value=fields.get("response_string")),
    ):
        asyncio.run(
            aporia.async_post_call_success_hook(  # pyright: ignore[reportUnknownMemberType]
                data=cast("dict[str, object]", data),
                user_api_key_dict=UserAPIKeyAuth(),
                response=response,
            )
        )
    return {"path": captured.get("path"), "request": captured.get("request")}


def guardrails_ai_construct(fields: dict[str, Json]) -> Json:
    environment = cast(dict[str, str], fields.get("environment") or {})
    with patch.dict(os.environ, environment, clear=False):
        guardrails_ai = GuardrailsAI(
            guard_name=cast(str, fields.get("guard_name")),
            api_base=cast("str | None", fields.get("api_base")),
        )
    return {
        "guard_name": guardrails_ai.guardrails_ai_guard_name,
        "api_base": guardrails_ai.guardrails_ai_api_base,
        "input_format": guardrails_ai.guardrails_ai_api_input_format,
    }


def guardrails_ai_process_input(fields: dict[str, Json]) -> Json:
    guardrails_ai = GuardrailsAI(
        guard_name=cast(str, fields["guard_name"]),
        api_base="https://guardrails-ai.test",
        guardrails_ai_api_input_format=cast(Literal["inputs", "llmOutput"], fields["input_format"]),
    )
    captured: dict[str, Json] = {}
    litellm.module_level_aclient.client = httpx.AsyncClient(
        transport=mock_transport(fields.get("answer"), captured)
    )
    data: dict[str, Json] = {}
    last_user_message = cast("str | None", fields.get("last_user_message"))
    if last_user_message is not None:
        data["messages"] = [{"role": "user", "content": last_user_message}]
    result = asyncio.run(  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]
        guardrails_ai.process_input(  # pyright: ignore[reportUnknownArgumentType, reportUnknownMemberType]
            data=cast("dict[str, object]", data), call_type="completion"
        )
    )
    updated_messages = cast(
        "list[dict[str, Json]] | None",
        result.get("messages"),  # pyright: ignore[reportUnknownMemberType]
    )
    updated_text = updated_messages[-1].get("content") if updated_messages else None
    return {
        "path": captured.get("path"),
        "request": captured.get("request"),
        "updated_text": updated_text,
    }


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    match operation:
        case "aporia_moderation":
            return aporia_moderation(fields)
        case "aporia_post_call":
            return aporia_post_call(fields)
        case "guardrails_ai_construct":
            return guardrails_ai_construct(fields)
        case "guardrails_ai_process_input":
            return guardrails_ai_process_input(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
