from typing import Literal, TypedDict, cast

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.llms.openai import OpenAIMessageContentListBlock
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class MapParams(TypedDict):
    function: Literal["map_params"]
    model: str
    model_cost: dict[str, JsonValue]
    environment: dict[str, str]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    global_drop: bool
    call_drop: bool


class CachePoint(TypedDict):
    function: Literal["cache_point"]
    model: str | None
    model_cost: dict[str, JsonValue]
    message_block: dict[str, JsonValue]
    control: dict[str, JsonValue] | None


class TopK(TypedDict):
    function: Literal["top_k"]
    model: str
    model_cost: dict[str, JsonValue]
    inference_params: dict[str, JsonValue]
    global_drop: bool
    call_drop: bool


class ParallelConfig(TypedDict):
    function: Literal["parallel_config"]
    additional_request_params: dict[str, JsonValue]
    parallel_tool_use_config: dict[str, JsonValue]


Input = MapParams | CachePoint | TopK | ParallelConfig


# LiteLLM's Converse methods are the seam; its block types carry Any, hence the pins.
class ConverseParamsConfig(AmazonConverseConfig):
    def compose(self, call: MapParams) -> dict[str, JsonValue]:
        return cast("dict[str, JsonValue]", self.map_openai_params(call["non_default_params"], call["optional_params"], call["model"], call["call_drop"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1010-L1165

    def cache_point_blocks(self, call: CachePoint) -> tuple[JsonValue, JsonValue]:
        block = cast("OpenAIMessageContentListBlock", call["message_block"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1320-L1338
        system = self.get_cache_point_block(block, "system", call["model"])  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1297-L1306
        content = self.get_cache_point_block(block, "content_block", call["model"])  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1309-L1318
        return JSON.validate_python(system), JSON.validate_python(content)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1320-L1338

    def cache_point_built(self, call: CachePoint) -> JsonValue:
        return JSON.validate_python(self._build_cache_point_block(call["control"], call["model"]))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1341-L1353

    def top_k_parts(
        self, call: TopK
    ) -> tuple[dict[str, JsonValue], dict[str, JsonValue], dict[str, JsonValue]]:
        remaining = PARAMS.validate_python(call["inference_params"])
        mapped = cast("dict[str, JsonValue]", self._handle_top_k_value(call["model"], remaining, call["call_drop"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1513-L1537
        transformed = cast("dict[str, JsonValue]", self._transform_inference_params(PARAMS.validate_python(call["inference_params"])))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1508-L1511
        return mapped, remaining, transformed

    def parallel_parts(
        self, call: ParallelConfig
    ) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
        merged = cast("dict[str, JsonValue]", self._merge_parallel_tool_use_config(call["additional_request_params"], call["parallel_tool_use_config"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1540-L1553
        dropped = PARAMS.validate_python(merged)
        self._drop_tool_choice_type_conflicting_with_tool_config(dropped)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1556-L1567
        return merged, dropped


def run(input_value: Input) -> JsonValue:
    config = ConverseParamsConfig()
    if input_value["function"] == "map_params":
        litellm.model_cost = input_value["model_cost"]
        litellm.drop_params = input_value["global_drop"]
        return {"params": config.compose(input_value)}
    if input_value["function"] == "cache_point":
        litellm.model_cost = input_value["model_cost"]
        system, content = config.cache_point_blocks(input_value)
        built = config.cache_point_built(input_value)
        return {"block": system, "content": content, "built": built}
    if input_value["function"] == "top_k":
        litellm.model_cost = input_value["model_cost"]
        litellm.drop_params = input_value["global_drop"]
        mapped, remaining, transformed = config.top_k_parts(input_value)
        return {"mapped": mapped, "remaining": remaining, "transformed": transformed}
    merged, dropped = config.parallel_parts(input_value)
    return {"merged": merged, "dropped": dropped}
