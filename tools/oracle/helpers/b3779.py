from collections.abc import Sequence
from typing import TypeAlias, cast

from litellm import ModelResponse
from litellm.proxy.guardrails.guardrail_hooks.lakera_ai_v2 import LakeraAIGuardrail  # pyright: ignore[reportMissingTypeStubs]
from litellm.proxy.guardrails.guardrail_hooks.lasso.lasso import LassoGuardrail  # pyright: ignore[reportMissingTypeStubs]
from litellm.types.llms.openai import AllMessageValues  # pyright: ignore[reportMissingTypeStubs]
from litellm.types.proxy.guardrails.guardrail_hooks.lakera_ai_v2 import LakeraAIResponse  # pyright: ignore[reportMissingTypeStubs]
from litellm.types.utils import ChatCompletionMessageCustomToolCall, ChatCompletionMessageToolCall  # pyright: ignore[reportMissingTypeStubs]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


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
        guardrail = LassoGuardrail(api_key="key", api_base="http://localhost")
        messages = cast(list[dict[str, object]], fields["messages"])
        return cast(Json, guardrail._expand_messages_for_classification(messages))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
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
