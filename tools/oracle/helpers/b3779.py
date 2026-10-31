import asyncio
from collections.abc import Sequence
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm import ModelResponse
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.lakera_ai_v2 import LakeraAIGuardrail  # pyright: ignore[reportMissingTypeStubs]
from litellm.proxy.guardrails.guardrail_hooks.lasso.lasso import LassoGuardrail  # pyright: ignore[reportMissingTypeStubs]
from litellm.types.llms.openai import AllMessageValues  # pyright: ignore[reportMissingTypeStubs]
from litellm.types.proxy.guardrails.guardrail_hooks.lakera_ai_v2 import LakeraAIResponse  # pyright: ignore[reportMissingTypeStubs]
from litellm.types.utils import ChatCompletionMessageCustomToolCall, ChatCompletionMessageToolCall  # pyright: ignore[reportMissingTypeStubs]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def post_call(fields: dict[str, Json]) -> Json:
    guardrail = LassoGuardrail(
        api_key="key",
        api_base="http://localhost",
        mask=bool(fields.get("mask", False)),
        conversation_id="oracle",
    )
    response = ModelResponse(**cast(dict[str, Json], fields["response"]))
    captured: dict[str, Json] = {}

    async def call_api(
        headers: dict[str, str], payload: dict[str, Json], api_url: str | None = None
    ) -> dict[str, Json]:
        del headers
        captured.update(
            endpoint="classifix" if api_url else "classify",
            message_type=payload["messageType"],
            inspected=payload["messages"],
        )
        return {
            "violations_detected": bool(fields.get("masked")),
            "messages": fields.get("masked", []),
        }

    with (
        patch.object(guardrail, "_call_lasso_api", call_api),
        patch.object(guardrail, "should_run_guardrail", return_value=True),
    ):
        asyncio.run(
            guardrail.async_post_call_success_hook(  # pyright: ignore[reportUnknownMemberType]
                data={}, user_api_key_dict=UserAPIKeyAuth(), response=response
            )
        )
    captured["response"] = [
        {
            "content": choice.message.content,
            "tool_calls": [tool_call_arguments(call) for call in (choice.message.tool_calls or [])],
        }
        for choice in response.choices
    ]
    return captured


def tool_call_arguments(
    call: ChatCompletionMessageToolCall | ChatCompletionMessageCustomToolCall,
) -> dict[str, Json]:
    match call:
        case ChatCompletionMessageCustomToolCall():
            return {"id": call.id, "arguments": call.custom.input}
        case ChatCompletionMessageToolCall():
            return {"id": call.id, "arguments": call.function.arguments}


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    expansion_operations = {
        "expand_messages_for_classification",
        "lifecycle_pre_call_hook",
        "lifecycle_post_call_hook",
    }
    if operation in expansion_operations:
        if operation == "lifecycle_post_call_hook":
            inspected = post_call(fields)
        else:
            guardrail = LassoGuardrail(api_key="key", api_base="http://localhost")
            messages = cast(list[dict[str, object]], fields["messages"])
            inspected = cast(Json, guardrail._expand_messages_for_classification(messages))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return inspected
    if operation == "map_masked_messages_back":
        guardrail = LassoGuardrail(api_key="key", api_base="http://localhost")
        original = cast(list[dict[str, object]], fields["original"])
        masked = cast(list[dict[str, object]], fields["masked"])
        return cast(Json, guardrail._map_masked_messages_back(original, masked))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if operation == "apply_masking_to_model_response":
        guardrail = LassoGuardrail(api_key="key", api_base="http://localhost")
        response = ModelResponse(**cast(dict[str, Json], fields["response"]))
        masked = cast(list[dict[str, object]], fields["masked"])
        guardrail._apply_masking_to_model_response(response, masked)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return [
            {
                "content": choice.message.content,
                "tool_calls": [
                    tool_call_arguments(call) for call in (choice.message.tool_calls or [])
                ],
            }
            for choice in response.choices
        ]
    if operation == "mask_pii_in_messages":
        guardrail = LakeraAIGuardrail(api_key="key", api_base="http://localhost")
        counts: dict[str, int] = {}
        messages = cast(Sequence[AllMessageValues], fields["messages"])
        response = cast(LakeraAIResponse, cast(object, fields["response"]))
        result = guardrail._mask_pii_in_messages(messages, response, counts)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        return cast(Json, {"messages": result, "counts": counts})
    if operation == "is_only_pii_violation":
        guardrail = LakeraAIGuardrail(api_key="key", api_base="http://localhost")
        response = cast(LakeraAIResponse, cast(object, fields["response"]))
        return guardrail._is_only_pii_violation(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if operation == "get_http_exception_for_blocked_guardrail":
        guardrail = LakeraAIGuardrail(api_key="key", api_base="http://localhost")
        response = cast(LakeraAIResponse, cast(object, fields["response"]))
        error = guardrail._get_http_exception_for_blocked_guardrail(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return {"status_code": error.status_code, "detail": error.detail}
    raise ValueError(f"unsupported operation {operation}")
