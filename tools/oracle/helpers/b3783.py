import asyncio
from types import SimpleNamespace
from typing import Literal, TypeAlias, cast

import httpx
from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.onyx.onyx import (
    OnyxGuardrail,  # pyright: ignore[reportMissingTypeStubs]
)
from litellm.types.utils import GenericGuardrailAPIInputs  # pyright: ignore[reportMissingTypeStubs]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    settings = cast(dict[str, Json], input_value.get("settings") or {})
    answer = input_value.get("answer")
    captured: dict[str, Json] = {}

    def handle(request: httpx.Request) -> httpx.Response:
        if answer is None:
            raise httpx.ReadTimeout("the mock never answers", request=request)
        captured["path"] = request.url.path
        captured["content_type"] = request.headers.get("content-type")
        captured["request"] = cast(Json, httpx.Response(200, content=request.content).json())
        reply = cast(dict[str, Json], answer)
        return httpx.Response(cast(int, reply["status"]), json=reply.get("body"))

    guardrail = OnyxGuardrail(
        api_key=cast("str | None", settings.get("api_key")),
        api_base=cast("str | None", settings.get("api_base")),
        timeout=cast("float | None", settings.get("timeout")),
    )
    guardrail.async_handler.client = httpx.AsyncClient(  # pyright: ignore[reportAttributeAccessIssue]
        transport=httpx.MockTransport(handle),
        timeout=guardrail.async_handler.client.timeout,  # pyright: ignore[reportAttributeAccessIssue]
    )
    logging_obj: SimpleNamespace | None = None
    call_id = input_value.get("litellm_call_id")
    if call_id is not None:
        logging_obj = SimpleNamespace(litellm_call_id=call_id)
    try:
        result = asyncio.run(
            guardrail.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
                inputs=cast(GenericGuardrailAPIInputs, input_value.get("inputs") or {}),
                request_data=cast("dict[str, object]", input_value.get("request_data") or {}),
                input_type=cast(
                    Literal["request", "response"], input_value.get("input_type", "request")
                ),
                logging_obj=logging_obj,  # pyright: ignore[reportArgumentType]
            )
        )
    except HTTPException as exc:
        return {
            "blocked": True,
            "status": exc.status_code,
            "path": captured.get("path"),
            "request": captured.get("request"),
            "content_type": captured.get("content_type"),
        }
    hooks = [hook.value for hook in OnyxGuardrail.get_supported_event_hooks()]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return {
        "supported_event_hooks": cast(Json, hooks),
        "path": captured.get("path"),
        "request": captured.get("request"),
        "content_type": captured.get("content_type"),
        "inputs": cast(Json, result),
    }
