"""Cisco conversation oracle adapter."""

# The pinned helper has private dynamic signatures; preserve JSON-shaped oracle values.
# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false, reportReturnType=false, reportArgumentType=false

from typing import TypeAlias

from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Guardrail = CiscoAIDefenseGuardrail


def run(input_value: dict[str, Json]) -> Json:  # noqa: PLR0911
    function = str(input_value.get("function"))
    value = input_value.get("value", input_value)
    if function.startswith("normalize_message_content"):
        return Guardrail._normalize_message_content(value)  # noqa: SLF001
    if function == "extract_message_tool_argument_parts":
        return Guardrail._extract_message_tool_argument_parts(value)  # noqa: SLF001
    if function == "extract_message_reasoning_parts":
        return Guardrail._extract_message_reasoning_parts(value)  # noqa: SLF001
    if function.startswith("flatten_responses_input"):
        return Guardrail._flatten_responses_input(value)  # noqa: SLF001
    if function.startswith("extract_inspect_messages_from_request"):
        return Guardrail._extract_inspect_messages_from_request(value)  # noqa: SLF001
    if function.startswith("extract_response_messages"):
        return Guardrail._extract_response_messages(value)  # noqa: SLF001
    if function == "extract_tool_definition_text":
        return Guardrail._extract_tool_definition_text(value)  # noqa: SLF001
    if function == "sanitized_messages_to_responses_input":
        return Guardrail._sanitized_messages_to_responses_input(value)  # noqa: SLF001
    if function == "rewrite_responses_input_text":
        source = value.get("input") if isinstance(value, dict) else None
        text = value.get("sanitized_text", "") if isinstance(value, dict) else ""
        return Guardrail._rewrite_responses_input_text(source, str(text))  # noqa: SLF001
    raise ValueError(f"unknown Cisco conversation function: {function}")
