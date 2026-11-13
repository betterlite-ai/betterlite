import asyncio
from http import HTTPStatus
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from fastapi import HTTPException
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.pangea.pangea import (  # pyright: ignore[reportMissingTypeStubs]
    PangeaHandler,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
API_BASE = "https://pangea.test"
COMPLETION_CALL_TYPES = ("text_completion", "atext_completion")
MISSING_ANSWER = "the mock never answers"


def mock_transport(
    answer: Json, captured: dict[str, Json], *, hanging: bool
) -> httpx.MockTransport:
    def handle(request: httpx.Request) -> httpx.Response:
        captured["path"] = request.url.path
        captured["request"] = cast(Json, httpx.Response(200, content=request.content).json())
        if hanging:
            raise httpx.ReadTimeout(MISSING_ANSWER, request=request)
        match answer:
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout(MISSING_ANSWER, request=request)

    return httpx.MockTransport(handle)


def build_handler(fields: dict[str, Json]) -> PangeaHandler:
    return PangeaHandler(
        guardrail_name=cast(str, fields["guardrail_name"]),
        api_key=cast(str, fields["api_key"]),
        api_base=API_BASE,
        pangea_input_recipe=cast("str | None", fields.get("input_recipe")),
        pangea_output_recipe=cast("str | None", fields.get("output_recipe")),
    )


def request_data(fields: dict[str, Json]) -> dict[str, Json]:
    data: dict[str, Json] = {}
    for key in ("messages", "prompt", "tools"):
        value = fields.get(key)
        if value is not None:
            data[key] = value
    return data


def capture(captured: dict[str, Json]) -> dict[str, Json]:
    return {"path": captured.get("path"), "request": captured.get("request")}


def blocked(captured: dict[str, Json], error: HTTPException) -> Json:
    if error.status_code != HTTPStatus.BAD_REQUEST:
        raise error
    return {
        **capture(captured),
        "blocked": True,
        "status": error.status_code,
        "detail": cast(Json, error.detail),
    }


def pre_call(fields: dict[str, Json], *, hanging: bool) -> Json:
    pangea = build_handler(fields)
    captured: dict[str, Json] = {}
    pangea.async_handler.client = httpx.AsyncClient(
        transport=mock_transport(fields.get("answer"), captured, hanging=hanging)
    )
    data = request_data(fields)
    call_type = cast(str, fields["call_type"])
    try:
        with patch.object(pangea, "should_run_guardrail", return_value=True):
            result = asyncio.run(  # pyright: ignore[reportUnknownVariableType]
                pangea.async_pre_call_hook(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    user_api_key_dict=UserAPIKeyAuth(),
                    cache=None,  # pyright: ignore[reportArgumentType]
                    data=cast("dict[str, object]", data),
                    call_type=call_type,
                )
            )
    except HTTPException as error:
        return blocked(captured, error)
    output = capture(captured)
    if result is not None:
        updated = cast(dict[str, Json], result)
        if call_type in COMPLETION_CALL_TYPES:
            output["prompt"] = updated.get("prompt")
        else:
            output["messages"] = updated.get("messages")
    return output


def post_call(fields: dict[str, Json]) -> Json:
    pangea = build_handler(fields)
    captured: dict[str, Json] = {}
    pangea.async_handler.client = httpx.AsyncClient(
        transport=mock_transport(fields.get("answer"), captured, hanging=False)
    )
    data = request_data(fields)
    choices = cast("list[Json]", fields.get("choices") or [])
    response: litellm.ModelResponse | litellm.TextCompletionResponse
    if fields.get("kind") == "text_completion":
        response = litellm.TextCompletionResponse(choices=[])
    else:
        response = litellm.ModelResponse(choices=[])
    object.__setattr__(response, "choices", choices)
    try:
        with patch.object(pangea, "should_run_guardrail", return_value=True):
            result = asyncio.run(  # pyright: ignore[reportUnknownVariableType]
                pangea.async_post_call_success_hook(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    data=cast("dict[str, object]", data),
                    user_api_key_dict=UserAPIKeyAuth(),
                    response=response,  # pyright: ignore[reportArgumentType]
                )
            )
    except HTTPException as error:
        return blocked(captured, error)
    output = capture(captured)
    if result is not None:
        updated = cast("litellm.ModelResponse | litellm.TextCompletionResponse", result)
        output["choices"] = cast(Json, updated.choices)
    return output


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    match operation:
        case "pre_call":
            return pre_call(fields, hanging=False)
        case "backend_timeout":
            return pre_call(fields, hanging=True)
        case "post_call":
            return post_call(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
