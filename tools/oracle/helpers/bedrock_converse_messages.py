from typing import Literal, TypedDict, cast

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Guarded(TypedDict):
    function: Literal["guarded"]
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]


class HandleOrphaned(TypedDict):
    function: Literal["handle_orphaned"]
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    neutralize_orphaned_blocks: bool
    modify_params: bool


class TransformSystem(TypedDict):
    function: Literal["transform_system"]
    messages: list[dict[str, JsonValue]]
    model: str | None
    model_cost: dict[str, JsonValue]


Input = Guarded | HandleOrphaned | TransformSystem


# LiteLLM's Converse methods carry Any in message records, hence the pinned ignores.
class ConverseMessagesConfig(AmazonConverseConfig):
    def guarded(self, call: Guarded) -> JsonValue:
        messages = cast("list[AllMessageValues]", call["messages"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional = dict(call["optional_params"])
        converted = self._convert_consecutive_user_messages_to_guarded_text(messages, optional)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L169-L218
        return JSON.validate_python({"messages": converted})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L169-L218

    def handle(self, call: HandleOrphaned) -> JsonValue:
        messages = cast("list[AllMessageValues]", call["messages"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params = dict(call["optional_params"])
        litellm.bedrock_neutralize_orphaned_tool_blocks = call["neutralize_orphaned_blocks"]
        litellm.modify_params = call["modify_params"]
        handled = self._handle_orphaned_tool_blocks(messages, optional_params)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L281-L296
        return JSON.validate_python({"messages": handled, "optional_params": optional_params})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L281-L296

    def system(self, call: TransformSystem) -> JsonValue:
        messages = cast("list[AllMessageValues]", call["messages"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        litellm.model_cost = call["model_cost"]
        converted, blocks = self._transform_system_message(messages, model=call["model"])  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1473-L1506
        return JSON.validate_python({"messages": converted, "system": blocks})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1473-L1506


def run(input_value: Input) -> JsonValue:
    config = ConverseMessagesConfig()
    if input_value["function"] == "guarded":
        return config.guarded(input_value)
    if input_value["function"] == "handle_orphaned":
        return config.handle(input_value)
    return config.system(input_value)
