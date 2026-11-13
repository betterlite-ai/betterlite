from dataclasses import dataclass
from typing import Final, Literal, TypeAlias, TypedDict, cast

import litellm
import litellm.utils
from litellm.llms.anthropic.pass_through.adapters import transformation
from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter,
)
from litellm.types.llms.openai import (
    ChatCompletionToolParamFunctionChunk as FunctionChunk,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JsonObject: TypeAlias = dict[str, Json]

ADAPTER: Final = LiteLLMAnthropicMessagesAdapter()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L352
ANSWER: Final = TypeAdapter(Json)


@dataclass
class FunctionProbe:
    provider_specific_fields: JsonObject


@dataclass
class ToolCallProbe:
    provider_specific_fields: JsonObject
    function: FunctionProbe


class ProbeFields(TypedDict):
    provider_specific_fields: JsonObject


class ProbeToolCall(TypedDict):
    provider_specific_fields: JsonObject
    function: ProbeFields


class TruncateCall(TypedDict):
    operation: Literal["truncate_tool_name"]
    name: str


class NativeToolCall(TypedDict):
    operation: Literal["is_provider_native_tool_dict"]
    tool: JsonObject


class FunctionToolCall(TypedDict):
    operation: Literal["is_openai_function_tool"]
    tool: JsonObject


class WebSearchCall(TypedDict):
    operation: Literal["is_web_search_tool"]
    tool: JsonObject


class ParamsCall(TypedDict):
    operation: Literal["translatable_params"]


class ChatToolCall(TypedDict):
    operation: Literal["chat_tool_param"]
    function: JsonObject
    tool: Json


class ToolCallSignatureCall(TypedDict):
    operation: Literal["tool_call_signature"]
    tool_call: ProbeToolCall
    attributes: bool


class ToolUseSignatureCall(TypedDict):
    operation: Literal["tool_use_signature"]
    content: JsonObject


class CompactionCall(TypedDict):
    operation: Literal["compaction_blocks"]
    fields: JsonObject


class MidSystemCall(TypedDict):
    operation: Literal["mid_system_supported"]
    model: str | None
    custom_llm_provider: str | None
    supported: list[str]


class CopyCacheCall(TypedDict):
    operation: Literal["copy_cache_control"]
    source: JsonObject
    target: JsonObject
    model: str | None


class CarryBreakpointCall(TypedDict):
    operation: Literal["carry_cache_breakpoint"]
    source: JsonObject
    target: JsonObject


class ConsumesCacheCall(TypedDict):
    operation: Literal["consumes_cache_control"]
    model: str


class ImageSourceCall(TypedDict):
    operation: Literal["image_source_url"]
    image_source: Json


class ImagePartCall(TypedDict):
    operation: Literal["tool_result_image_part"]
    image_source: Json


class ResultPartCall(TypedDict):
    operation: Literal["tool_result_part"]
    item: Json


class ResultContentCall(TypedDict):
    operation: Literal["tool_result_content"]
    raw_content: Json


ToolsGroup: TypeAlias = (
    TruncateCall | NativeToolCall | FunctionToolCall | WebSearchCall | ParamsCall | ChatToolCall
)
SignatureGroup: TypeAlias = (
    ToolCallSignatureCall | ToolUseSignatureCall | CompactionCall | MidSystemCall
)
CacheGroup: TypeAlias = CopyCacheCall | CarryBreakpointCall | ConsumesCacheCall
ResultGroup: TypeAlias = ImageSourceCall | ImagePartCall | ResultPartCall | ResultContentCall
Input: TypeAlias = ToolsGroup | SignatureGroup | CacheGroup | ResultGroup


def tools_answer(call: ToolsGroup) -> Json:
    if call["operation"] == "truncate_tool_name":
        return transformation.truncate_tool_name(call["name"])
    if call["operation"] == "is_provider_native_tool_dict":
        return transformation.is_provider_native_tool_dict(call["tool"])
    if call["operation"] == "is_openai_function_tool":
        return transformation._is_openai_function_tool(call["tool"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L62
    if call["operation"] == "is_web_search_tool":
        return ADAPTER._is_web_search_tool(call["tool"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L436
    if call["operation"] == "translatable_params":
        return list[Json](ADAPTER.translatable_anthropic_params())
    chunk = cast(FunctionChunk, call["function"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L588
    param = transformation._chat_tool_param(chunk, call["tool"])  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L219
    return ANSWER.validate_python(param)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L219


def mid_system_answer(call: MidSystemCall) -> Json:
    flagged: dict[str, JsonObject] = {
        name: {
            "litellm_provider": call["custom_llm_provider"],
            "mode": "chat",
            "supports_mid_conversation_system": True,
        }
        for name in call["supported"]
    }
    previous_cost = litellm.model_cost  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    previous_bundled = litellm.utils._get_bundled_model_cost_map  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
    litellm.model_cost = flagged
    # no public entry injects the packaged cost map, so the fixture replaces the private reader
    litellm.utils._get_bundled_model_cost_map = dict  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
    try:
        return transformation.target_supports_mid_conversation_system(
            call["model"], call["custom_llm_provider"]
        )
    finally:
        litellm.model_cost = previous_cost
        litellm.utils._get_bundled_model_cost_map = previous_bundled  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419


def signature_answer(call: SignatureGroup) -> Json:
    if call["operation"] == "tool_call_signature":
        fields = call["tool_call"]
        # a raw mapping has no attributes, so only the probe carries them
        probe = ToolCallProbe(
            fields["provider_specific_fields"],
            FunctionProbe(fields["function"]["provider_specific_fields"]),
        )
        if call["attributes"]:
            return ADAPTER._extract_signature_from_tool_call(probe)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L357
        return ADAPTER._extract_signature_from_tool_call(fields)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L357
    if call["operation"] == "tool_use_signature":
        return ADAPTER._extract_signature_from_tool_use_content(call["content"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L372
    if call["operation"] == "compaction_blocks":
        blocks = transformation._compaction_blocks(call["fields"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L43
        return ANSWER.validate_python(list(blocks))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L43
    return mid_system_answer(call)


def cache_answer(call: CacheGroup) -> Json:
    if call["operation"] == "copy_cache_control":
        copied = dict(call["target"])
        ADAPTER._add_cache_control_if_applicable(call["source"], copied, call["model"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L382
        return copied
    if call["operation"] == "carry_cache_breakpoint":
        return ANSWER.validate_python(
            ADAPTER._add_prompt_cache_breakpoint_if_present(call["source"], call["target"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L415
        )
    return ADAPTER.target_consumes_cache_control(call["model"])


def result_answer(call: ResultGroup) -> Json:
    if call["operation"] == "image_source_url":
        source = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L1290
            "dict[str, str]", call["image_source"]
        )
        return ADAPTER._translate_anthropic_image_to_openai(source)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L1290
    if call["operation"] == "tool_result_image_part":
        part = ADAPTER._tool_result_image_part(call["image_source"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L1337
        return ANSWER.validate_python(part)
    if call["operation"] == "tool_result_part":
        item = ADAPTER._tool_result_part(call["item"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L1319
        return ANSWER.validate_python(item)
    content = ADAPTER._tool_result_content(call["raw_content"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L1304
    return ANSWER.validate_python(content)


def run(input_value: Input) -> Json:
    match input_value["operation"]:
        case (
            "truncate_tool_name"
            | "is_provider_native_tool_dict"
            | "is_openai_function_tool"
            | "is_web_search_tool"
            | "translatable_params"
            | "chat_tool_param"
        ):
            return tools_answer(input_value)
        case (
            "tool_call_signature"
            | "tool_use_signature"
            | "compaction_blocks"
            | "mid_system_supported"
        ):
            return signature_answer(input_value)
        case "copy_cache_control" | "carry_cache_breakpoint" | "consumes_cache_control":
            return cache_answer(input_value)
        case _:
            return result_answer(input_value)
