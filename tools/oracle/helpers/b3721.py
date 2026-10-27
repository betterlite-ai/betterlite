import json
from collections.abc import Callable
from typing import Literal, TypeAlias, cast

from litellm import ModelResponse
from litellm.proxy.guardrails.guardrail_hooks.bedrock_guardrails import BedrockGuardrail
from litellm.types.llms.openai import AllMessageValues
from litellm.types.proxy.guardrails.guardrail_hooks.bedrock_guardrails import (
    BedrockGuardrailResponse,
)
from litellm.types.utils import (
    Choices,
    Delta,
    GenericGuardrailAPIInputs,
    StreamingChoices,
    TextChoices,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class InputError(ValueError):
    pass


def model_response(wire: dict[str, Json]) -> ModelResponse:
    choices: list[Choices | StreamingChoices | TextChoices] = []
    for choice in cast(list[dict[str, Json]], wire.get("choices", [])):
        if "message" in choice:
            choices.append(Choices(message=cast(dict[str, Json], choice["message"])))
        elif "delta" in choice:
            choices.append(StreamingChoices(delta=Delta.model_validate(choice["delta"])))
        else:
            choices.append(TextChoices(text=choice.get("text")))
    response = ModelResponse()
    object.__setattr__(response, "choices", choices)
    return response


def mask_response(guardrail: BedrockGuardrail, arguments: dict[str, Json]) -> Json:
    original = cast(dict[str, Json], arguments["response"])
    model = model_response(original)
    guardrail._apply_masking_to_model_response(model, cast(list[str], arguments["masked_texts"]))  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    for source, choice in zip(
        cast(list[dict[str, Json]], original["choices"]),
        cast(list[Choices | StreamingChoices | TextChoices], model.choices),
        strict=True,
    ):
        if isinstance(choice, Choices):
            cast(dict[str, Json], source["message"])["content"] = choice.message.content
        elif isinstance(choice, StreamingChoices):
            cast(dict[str, Json], source["delta"])["content"] = choice.delta.content
        else:
            source["text"] = cast(str | None, choice.text)
    return original


def run(input_value: dict[str, Json]) -> Json:
    ((operation, arguments),) = cast(dict[str, dict[str, Json]], input_value["step"]).items()
    guardrail = BedrockGuardrail.__new__(BedrockGuardrail)
    guardrail.contextual_grounding_from_messages = bool(arguments.get("contextual_fallback", False))
    messages = cast(list[AllMessageValues] | None, arguments.get("messages"))
    result: object
    if operation == "format":
        response_wire = cast(dict[str, Json] | None, arguments.get("response"))
        response = model_response(response_wire) if response_wire is not None else None
        result = guardrail.convert_to_bedrock_format(
            cast(Literal["INPUT", "OUTPUT"], arguments["source"]), messages, response
        )
    elif operation == "grounding":
        blocks = guardrail._collect_grounding_blocks(messages)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
        result = [block._asdict() for block in blocks]
    elif operation == "selection":
        context = cast(dict[str, Json], arguments["context"])
        guardrail.experimental_use_latest_role_message_only = bool(
            context.get("latest_only", False)
        )
        object.__setattr__(
            guardrail, "scan_only_tool_results", bool(context.get("tool_results_only", False))
        )
        inputs = cast(
            GenericGuardrailAPIInputs, {"structured_messages": context.get("structured_messages")}
        )
        selected = guardrail._select_messages_for_apply_guardrail(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            cast(list[str], arguments["texts"]),
            inputs,
            {"messages": context.get("messages")},
            cast(Literal["request", "response"], context.get("scan", "request")),
        )
        result = selected._asdict()
    elif operation == "prepare":
        guardrail.experimental_use_latest_role_message_only = bool(arguments["latest_only"])
        result = guardrail._prepare_guardrail_messages_for_role(messages)._asdict()  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    elif operation == "merge":
        merge = cast(Callable[[list[str], list[str], tuple[int, int] | None, bool], list[str]], guardrail._merge_masked_texts)  # noqa: E501, SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        result = merge(
            cast(list[str], arguments["masked"]),
            cast(list[str], arguments["texts"]),
            cast(tuple[int, int] | None, arguments.get("scanned_slice")),
            bool(arguments["scanned_role_subset"]),
        )
    elif operation == "merge_messages":
        result = guardrail._merge_filtered_messages(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(list[AllMessageValues] | None, arguments.get("original")),
            cast(list[AllMessageValues], arguments["updated"]),
            cast(list[int] | None, arguments.get("indices")),
        )
    elif operation == "extract":
        result = guardrail._extract_masked_texts_from_response(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(BedrockGuardrailResponse, arguments.get("response") or {})
        )
    elif operation == "mask_messages":
        result = guardrail._apply_masking_to_messages(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            messages or [], cast(list[str], arguments["masked_texts"])
        )
    elif operation == "mask_response":
        result = mask_response(guardrail, arguments)
    else:
        raise InputError(f"unsupported operation {operation}")
    return cast(Json, json.loads(json.dumps(result)))
