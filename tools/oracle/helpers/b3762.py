import asyncio
import copy
from collections.abc import Mapping
from typing import Literal, TypeAlias, cast

import httpx
from fastapi import HTTPException
from litellm import DualCache, ModelResponse
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def guardrail(settings: dict[str, Json]) -> CiscoAIDefenseGuardrail:
    return CiscoAIDefenseGuardrail(
        guardrail_name="cisco-ai-defense",
        **settings,  # pyright: ignore[reportArgumentType]
    )


def construct(settings: dict[str, Json]) -> Json:
    cisco = guardrail(settings)
    return cast(
        Json,
        {
            "guardrail_name": cisco.guardrail_name,
            "api_base": cisco.api_base,
            "inspect_path": cisco.inspect_path,
            "inspection_type": cisco.inspection_type,
            "on_flagged_action": cisco.on_flagged_action,
            "fallback_on_error": cisco.fallback_on_error,
            "timeout_seconds": cisco.timeout,
        },
    )


def headers(settings: dict[str, Json]) -> Json:
    cisco = guardrail(settings)
    raw = cisco._build_headers()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"headers": {name.lower(): value for name, value in raw.items()}}


def install_mock(cisco: CiscoAIDefenseGuardrail, answer: Json) -> tuple[list[str], list[Json]]:
    paths: list[str] = []
    bodies: list[Json] = []

    def handle(request: httpx.Request) -> httpx.Response:
        paths.append(request.url.path)
        bodies.append(cast(Json, httpx.Response(200, content=request.content).json()))
        match answer:
            case {"status": int(status), "body": None}:
                return httpx.Response(status)
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    cisco.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    return paths, bodies


def response_with_redacted_content(original: dict[str, Json], response_obj: ModelResponse) -> Json:
    # async_post_call_success_hook mutates response_obj's Message.content in place; the
    # dict handed to ModelResponse(**response) never sees that mutation, so the caller
    # has to read the redacted text back off response_obj itself.
    mutated = copy.deepcopy(original)
    choices = mutated.get("choices")
    if not isinstance(choices, list):
        return cast(Json, mutated)
    for index, choice in enumerate(choices):
        if not isinstance(choice, dict) or index >= len(response_obj.choices):
            continue
        message = choice.get("message")
        if not isinstance(message, dict):
            continue
        mutated_message = getattr(response_obj.choices[index], "message", None)
        message["content"] = getattr(mutated_message, "content", message.get("content"))
    return cast(Json, mutated)


def applied_guardrails_list(data: dict[str, Json]) -> list[str]:
    bucket_key = "litellm_metadata" if "litellm_metadata" in data else "metadata"
    bucket = data.get(bucket_key)
    if not isinstance(bucket, dict):
        return []
    names = bucket.get("applied_guardrails")
    if not isinstance(names, list):
        return []
    return [name for name in names if isinstance(name, str)]


def outcome_tag(result: Mapping[str, object]) -> Json:
    if not result:
        return None
    if result.get("_unscanned"):
        return {"tag": "unscanned"}
    return {"tag": "allow"}


def api_error_tag(error: HTTPException) -> Json:
    return {
        "tag": "api_error_blocked",
        "detail": cast(Json, error.detail),
    }


def user_api_key(key_user_id: str | None) -> UserAPIKeyAuth:
    if key_user_id is None:
        return UserAPIKeyAuth()
    return UserAPIKeyAuth(user_id=key_user_id)


def call_type_of(fields: dict[str, Json]) -> Literal["completion", "mcp_call"]:
    return cast(
        'Literal["completion", "mcp_call"]', fields.get("call_type") or "completion"
    )


async def pre_call_hook(fields: dict[str, Json], is_moderation: bool) -> Json:  # noqa: FBT001
    cisco = guardrail(cast(dict[str, Json], fields["settings"]))
    paths, bodies = install_mock(cisco, fields.get("answer"))
    should_run = cast(bool, fields.get("should_run", True))
    cisco.should_run_guardrail = (  # pyright: ignore[reportAttributeAccessIssue]
        lambda data, event_type: should_run  # noqa: ARG005  # pyright: ignore[reportUnknownLambdaType]
    )
    data = cast(dict[str, Json], fields["request_data"])
    call_type = call_type_of(fields)
    key_dict = user_api_key(cast("str | None", fields.get("key_user_id")))
    try:
        if is_moderation:
            await cisco.async_moderation_hook(  # pyright: ignore[reportUnknownMemberType]
                data=cast("dict[str, object]", data),
                user_api_key_dict=key_dict,
                call_type=call_type,
            )
        else:
            await cisco.async_pre_call_hook(  # pyright: ignore[reportUnknownMemberType]
                user_api_key_dict=key_dict,
                cache=DualCache(),
                data=cast("dict[str, object]", data),
                call_type=call_type,
            )
    except HTTPException as error:
        outcome: Json = api_error_tag(error)
    else:
        ran = cisco.guardrail_name in applied_guardrails_list(data)
        outcome = {"tag": "allow"} if ran else None
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "outcome": outcome,
        "applied_guardrails": cast(Json, applied_guardrails_list(data)),
    }


async def post_call_hook(fields: dict[str, Json]) -> Json:
    cisco = guardrail(cast(dict[str, Json], fields["settings"]))
    paths, bodies = install_mock(cisco, fields.get("answer"))
    should_run = cast(bool, fields.get("should_run", True))
    cisco.should_run_guardrail = (  # pyright: ignore[reportAttributeAccessIssue]
        lambda data, event_type: should_run  # noqa: ARG005  # pyright: ignore[reportUnknownLambdaType]
    )
    data = cast(dict[str, Json], fields["request_data"])
    response = cast(dict[str, Json], fields["response"])
    response_obj = ModelResponse(**cast("dict[str, object]", response))
    key_dict = user_api_key(cast("str | None", fields.get("key_user_id")))
    try:
        await cisco.async_post_call_success_hook(  # pyright: ignore[reportUnknownMemberType]
            data=cast("dict[str, object]", data),
            user_api_key_dict=key_dict,
            response=response_obj,
        )
    except HTTPException as error:
        outcome: Json = api_error_tag(error)
    else:
        ran = cisco.guardrail_name in applied_guardrails_list(data)
        outcome = {"tag": "allow"} if ran else None
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "outcome": outcome,
        "applied_guardrails": cast(Json, applied_guardrails_list(data)),
        "response": response_with_redacted_content(response, response_obj),
    }


async def inspect_chat(fields: dict[str, Json]) -> Json:
    cisco = guardrail(cast(dict[str, Json], fields["settings"]))
    paths, bodies = install_mock(cisco, fields.get("answer"))
    data = cast(dict[str, Json], fields["request_data"])
    direction = cast(str, fields.get("direction") or "input")
    messages = cast("list[dict[str, str]]", data.get("messages") or [])
    key_dict = user_api_key(cast("str | None", fields.get("key_user_id")))
    try:
        result = await cisco._inspect_chat(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            messages=messages,
            request_data=cast("dict[str, object]", data),
            user_api_key_dict=key_dict,
            direction=direction,
        )
    except HTTPException as error:
        outcome: Json = api_error_tag(error)
    else:
        outcome = outcome_tag(result)
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "outcome": outcome,
    }


async def inspect_mcp_request(fields: dict[str, Json]) -> Json:
    cisco = guardrail(cast(dict[str, Json], fields["settings"]))
    paths, bodies = install_mock(cisco, fields.get("answer"))
    data = cast(dict[str, Json], fields["request_data"])
    try:
        result = await cisco._inspect_mcp_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            data=cast("dict[str, object]", data), user_api_key_dict=UserAPIKeyAuth()
        )
    except HTTPException as error:
        outcome: Json = api_error_tag(error)
    else:
        outcome = outcome_tag(result)
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "outcome": outcome,
    }


async def inspect_mcp_response(fields: dict[str, Json]) -> Json:
    cisco = guardrail(cast(dict[str, Json], fields["settings"]))
    paths, bodies = install_mock(cisco, fields.get("answer"))
    data = cast(dict[str, Json], fields["request_data"])
    response = cast("object", fields["response"])
    try:
        result = await cisco._inspect_mcp_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            request_data=cast("dict[str, object]", data), response=response
        )
    except HTTPException as error:
        outcome: Json = api_error_tag(error)
    else:
        outcome = outcome_tag(result)
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "outcome": outcome,
    }


async def post_inspection(fields: dict[str, Json]) -> Json:
    cisco = guardrail(cast(dict[str, Json], fields["settings"]))
    paths, bodies = install_mock(cisco, fields.get("answer"))
    surface = cast(str, fields.get("surface") or "chat")
    payload = cast("dict[str, object]", fields.get("payload") or {})
    url = f"{cisco.api_base}{cisco.inspect_path}"
    result = await cisco._post_inspection(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        url=url, payload=payload, surface=surface
    )
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "result": cast(Json, result),
    }


def run(input_value: dict[str, Json]) -> Json:  # noqa: PLR0911 - one branch per b3762 operation
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    match operation:
        case "construct":
            return construct(cast(dict[str, Json], fields["settings"]))
        case "build_headers":
            return headers(cast(dict[str, Json], fields["settings"]))
        case "pre_call_hook":
            return asyncio.run(pre_call_hook(fields, is_moderation=False))
        case "moderation_hook":
            return asyncio.run(pre_call_hook(fields, is_moderation=True))
        case "post_call_hook":
            return asyncio.run(post_call_hook(fields))
        case "inspect_chat":
            return asyncio.run(inspect_chat(fields))
        case "inspect_mcp_request":
            return asyncio.run(inspect_mcp_request(fields))
        case "inspect_mcp_response":
            return asyncio.run(inspect_mcp_response(fields))
        case "post_inspection":
            return asyncio.run(post_inspection(fields))
        case _:
            raise ValueError(f"unsupported operation {operation}")
