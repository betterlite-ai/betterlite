import asyncio
from typing import TypeAlias, cast

import httpx
import litellm.proxy.guardrails.guardrail_hooks.generic_guardrail_api.generic_guardrail_api as generic_guardrail_api
import litellm.proxy.guardrails.guardrail_hooks.grayswan.grayswan as grayswan
from litellm.exceptions import ModifyResponseException

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
VIOLATION_THRESHOLD = 0.5


def metadata(data: dict[str, Json]) -> Json:
    api = generic_guardrail_api.GenericGuardrailAPI(api_base="http://oracle.invalid")
    return cast(Json, api._extract_user_api_key_metadata(cast(dict, data)))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def grayswan_verdict(data: dict[str, Json]) -> Json:
    action = cast(str, data.get("action", "BLOCK")).lower()
    score = float(cast(float, data.get("score", 0.0)))
    is_output = bool(data.get("is_output", False))
    instance = grayswan.GraySwanGuardrail(
        api_key="oracle-key", on_flagged_action=action, violation_threshold=VIOLATION_THRESHOLD
    )
    response = {"violation": score, "violated_rule_descriptions": [], "mutation": None, "ipi": None}
    try:
        instance._process_response_internal(response, {}, {"texts": ["hello"]}, is_output)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except ModifyResponseException:
        return {"allowed": True, "flagged": True}
    return {"allowed": True, "flagged": score >= VIOLATION_THRESHOLD}


def generic_call(fields: dict[str, Json]) -> Json:
    answer = cast(dict[str, Json], fields.get("answer")) if fields.get("answer") is not None else None
    unreachable_fallback = cast(str, fields.get("unreachable_fallback", "fail_closed"))
    fail_on_error = cast(bool, fields.get("fail_on_error", True))
    driven_inputs = cast(dict[str, Json] | None, fields.get("inputs"))
    inputs: dict[str, Json] = dict(driven_inputs) if driven_inputs is not None else {"texts": ["hello"]}

    def handle(request: httpx.Request) -> httpx.Response:
        if answer is not None:
            return httpx.Response(cast(int, answer["status"]), json=cast(Json, answer.get("body")))
        raise httpx.ReadTimeout("the mock never answers", request=request)

    async def call() -> Json:
        api = generic_guardrail_api.GenericGuardrailAPI(
            api_base="http://oracle.invalid",
            unreachable_fallback=unreachable_fallback,
            fail_on_error=fail_on_error,
        )
        api.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
        result = await api.apply_guardrail(inputs=cast(Json, inputs), request_data={}, input_type="request")
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
    if "metadata" in data or "litellm_metadata" in data:
        return metadata(data)
    if "score" in data or "action" in data:
        return grayswan_verdict(data)
    raise ValueError("unsupported operation")
