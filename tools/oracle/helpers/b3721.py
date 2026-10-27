import json
from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.bedrock_guardrails import BedrockGuardrail

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class InputError(ValueError):
    pass


def run(input_value: dict[str, Json]) -> Json:
    ((operation, arguments),) = cast(dict[str, dict[str, Json]], input_value["step"]).items()
    guardrail = BedrockGuardrail.__new__(BedrockGuardrail)
    result: object
    if operation == "input":
        result = guardrail._create_bedrock_input_content_request(cast(list[dict[str, Json]], arguments["messages"]))  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage]
    elif operation == "grounding":
        result = guardrail._collect_grounding_blocks(cast(list[dict[str, Json]], arguments["messages"]))  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage]
    elif operation == "merge":
        result = guardrail._merge_masked_texts(cast(list[str], arguments["masked"]), cast(list[str], arguments["texts"]), arguments.get("scanned_slice"), bool(arguments["scanned_role_subset"]))  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage]
    elif operation == "extract":
        result = guardrail._extract_masked_texts_from_response(cast(dict[str, Json], arguments.get("response") or {}))  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage]
    elif operation == "mask_messages":
        result = guardrail._apply_masking_to_messages(cast(list[dict[str, Json]], arguments["messages"]), cast(list[str], arguments["masked_texts"]))  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage]
    elif operation == "mask_response":
        response = cast(dict[str, Json], arguments["response"])
        guardrail._apply_masking_to_model_response(response, cast(list[str], arguments["masked_texts"]))  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage]
        result = response
    else:
        raise InputError(f"unsupported operation {operation}")
    return cast(Json, json.loads(json.dumps(result)))
