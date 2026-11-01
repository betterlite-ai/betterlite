import asyncio
import json
from collections.abc import Mapping
from typing import TYPE_CHECKING, Literal, TypeAlias, cast

import httpx
from litellm.exceptions import ModifyResponseException
from litellm.proxy.guardrails.guardrail_hooks.generic_guardrail_api import generic_guardrail_api
from litellm.proxy.guardrails.guardrail_hooks.grayswan import grayswan
from litellm.types.utils import GenericGuardrailAPIInputs

if TYPE_CHECKING:
    from litellm.proxy.guardrails.guardrail_hooks.grayswan.grayswan import (
        _GraySwanMonitorResponse,  # pyright: ignore[reportPrivateUsage]
    )
    from litellm.types.proxy.guardrails.guardrail_hooks.generic_guardrail_api import (
        GenericGuardrailAPIMetadata,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
VIOLATION_THRESHOLD = 0.5


def metadata(data: dict[str, Json]) -> Json:
    api = generic_guardrail_api.GenericGuardrailAPI(api_base="http://oracle.invalid")
    result: GenericGuardrailAPIMetadata = api._extract_user_api_key_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        data
    )
    return cast(Json, result)


def grayswan_verdict(data: dict[str, Json]) -> Json:
    action = cast(str, data.get("action", "BLOCK")).lower()
    score = float(cast(float, data.get("score", 0.0)))
    is_output = bool(data.get("is_output", False))
    instance = grayswan.GraySwanGuardrail(
        api_key="oracle-key", on_flagged_action=action, violation_threshold=VIOLATION_THRESHOLD
    )
    response: _GraySwanMonitorResponse = {
        "violation": score,
        "violated_rule_descriptions": [],
        "mutation": None,
        "ipi": None,
    }
    try:
        result = instance._process_response_internal(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            response, {}, {"texts": ["hello"]}, is_output
        )
    except ModifyResponseException:
        return {"allowed": True, "flagged": True, "replaced": True}
    return {
        "allowed": True,
        "flagged": score >= VIOLATION_THRESHOLD,
        "replaced": cast(Json, result.get("texts")) != ["hello"],
    }


def grayswan_payload(fields: dict[str, Json]) -> Json:
    config = cast(dict[str, Json], fields.get("config") or {})
    dynamic = cast(dict[str, Json], fields.get("dynamic") or {})
    messages = cast(
        list[Mapping[str, object]],
        fields.get("messages") or [{"role": "user", "content": "hello"}],
    )
    tools = cast(list[object], fields.get("tools") or [])
    instance = grayswan.GraySwanGuardrail(
        api_key="oracle-key",
        categories=cast("dict[str, str] | None", config.get("categories")),
        policy_id=cast("str | None", config.get("policy_id")),
        reasoning_mode=cast("str | None", config.get("reasoning_mode")),
    )
    payload = instance._prepare_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        tuple(messages), dict(dynamic), {}, None, tools=tuple(tools) or None
    )
    return cast(Json, payload)


def grayswan_call(fields: dict[str, Json]) -> Json:
    input_type = cast(Literal["request", "response"], fields.get("input_type", "request"))
    texts = cast(list[str], fields.get("texts") or [])
    tool_calls = cast(list[object], fields.get("tool_calls") or [])
    context = cast(list[Mapping[str, object]], fields.get("context") or [])
    action = cast(str, fields.get("action", "MONITOR"))
    violation_threshold = float(cast(float, fields.get("violation_threshold", VIOLATION_THRESHOLD)))
    fail_open = bool(fields.get("fail_open", True))
    answer = (
        cast(dict[str, Json], fields.get("answer")) if fields.get("answer") is not None else None
    )
    fail = cast(str, fields.get("fail", "timeout"))
    captured: dict[str, Json] = {"payload": None}

    def handle(request: httpx.Request) -> httpx.Response:
        captured["payload"] = cast(Json, json.loads(request.content))
        if answer is not None:
            return httpx.Response(cast(int, answer["status"]), json=answer.get("body"))
        if fail == "closed":
            raise httpx.ConnectError("mock connection closed", request=request)
        raise httpx.ReadTimeout("the mock never answers", request=request)

    async def call() -> Json:
        instance = grayswan.GraySwanGuardrail(
            api_key="oracle-key",
            on_flagged_action=action,
            violation_threshold=violation_threshold,
            fail_open=fail_open,
        )
        instance._post_call_context = lambda *_a, **_kw: (  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownLambdaType]
            tuple(context),
            None,
        )
        instance.async_handler.client = httpx.AsyncClient(  # pyright: ignore[reportAttributeAccessIssue]
            transport=httpx.MockTransport(handle)
        )
        inputs: GenericGuardrailAPIInputs = {"texts": texts}
        if tool_calls:
            inputs["tool_calls"] = tool_calls  # pyright: ignore[reportGeneralTypeIssues]
        await instance.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
            inputs=inputs, request_data={}, input_type=input_type
        )
        return captured["payload"]

    return asyncio.run(call())


def generic_call(fields: dict[str, Json]) -> Json:
    answer = (
        cast(dict[str, Json], fields.get("answer")) if fields.get("answer") is not None else None
    )
    unreachable_fallback = cast(
        Literal["fail_closed", "fail_open"],
        fields.get("unreachable_fallback", "fail_closed"),
    )
    fail_on_error = cast(bool, fields.get("fail_on_error", True))
    driven_inputs = cast(dict[str, Json] | None, fields.get("inputs"))
    inputs: GenericGuardrailAPIInputs = (
        cast(GenericGuardrailAPIInputs, driven_inputs)
        if driven_inputs is not None
        else {"texts": ["hello"]}
    )

    def handle(request: httpx.Request) -> httpx.Response:
        if answer is not None:
            return httpx.Response(cast(int, answer["status"]), json=answer.get("body"))
        raise httpx.ReadTimeout("the mock never answers", request=request)

    async def call() -> Json:
        api = generic_guardrail_api.GenericGuardrailAPI(
            api_base="http://oracle.invalid",
            unreachable_fallback=unreachable_fallback,
            fail_on_error=fail_on_error,
        )
        api.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
        result = await api.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
            inputs=inputs, request_data={}, input_type="request"
        )
        return {
            "texts": cast(Json, result.get("texts")),
            "images": cast(Json, result.get("images")),
            "tools": cast(Json, result.get("tools")),
            "structured_messages": cast(Json, result.get("structured_messages")),
            "blocked": False,
        }

    return asyncio.run(call())


def run(input_value: dict[str, Json]) -> Json:
    data = input_value
    if "generic_call" in data:
        return generic_call(cast(dict[str, Json], data["generic_call"]))
    if "grayswan_call" in data:
        return grayswan_call(cast(dict[str, Json], data["grayswan_call"]))
    if "grayswan_payload" in data:
        return grayswan_payload(cast(dict[str, Json], data["grayswan_payload"]))
    if "metadata" in data or "litellm_metadata" in data:
        return metadata(data)
    if "score" in data or "action" in data:
        return grayswan_verdict(data)
    raise ValueError("unsupported operation")
