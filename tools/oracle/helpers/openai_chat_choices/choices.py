from typing import cast
from unittest import mock

import litellm.types.utils
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from litellm.types.llms.openai import OpenAIChatCompletionChoices
from litellm.types.utils import Choices

from . import Json, Object, Step
from .support import FixedUuid, as_object


def run(steps: list[Step]) -> Json:
    config = OpenAIGPTConfig()
    outputs: list[Json] = []
    with mock.patch("litellm.types.utils.uuid.uuid4", new=FixedUuid()):
        for step in steps:
            raw = cast(list[OpenAIChatCompletionChoices], step["choices"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L2407-L2411
            result = config._transform_choices(raw, step.get("json_mode"), step.get("optional_params"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L588-L659
            outputs.append([choice_view(choice) for choice in result])
    return outputs


def choice_view(choice: Choices) -> Json:
    dumped = as_object(choice.model_dump())  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1644-L1697
    view: Object = {
        "index": dumped.get("index"),
        "finish_reason": dumped.get("finish_reason"),
        "message": message_view(dumped.get("message")),
    }
    native = dumped.get("provider_specific_fields")
    if native is not None:
        view["provider_specific_fields"] = native
    return view


def message_view(message: Json) -> Json:
    fields = as_object(message)
    view: Object = {
        "role": fields.get("role"),
        "content": fields.get("content"),
        "tool_calls": tool_calls_view(fields.get("tool_calls")),
    }
    reasoning = fields.get("reasoning_content")
    if reasoning is not None:
        view["reasoning_content"] = reasoning
    return view


def tool_calls_view(calls: Json) -> Json:
    if calls is None:
        return None
    return [tool_call_view(call) for call in cast(list[Json], calls)]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L618-L622


def tool_call_view(call: Json) -> Json:
    fields = as_object(call)
    kind = fields.get("type")
    if kind == "custom":
        return {"id": fields.get("id"), "type": kind, "custom": fields.get("custom")}
    return {"id": fields.get("id"), "type": kind, "function": fields.get("function")}

