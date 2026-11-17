from typing import TypedDict, cast

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from pydantic import JsonValue


class Prepare(TypedDict):
    model: str
    model_cost: dict[str, JsonValue]
    tools: list[dict[str, JsonValue]]
    headers: dict[str, JsonValue] | None
    params: dict[str, JsonValue]


Input = Prepare


class BetaConfig(AmazonConverseConfig):
    def prepare(self, call: Prepare) -> tuple[JsonValue, JsonValue]:
        return cast("tuple[JsonValue, JsonValue]", self._process_tools_and_beta(call["tools"], call["model"], call["headers"], call["params"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1686-L1827


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    tools, betas = BetaConfig().prepare(input_value)
    return {"tools": tools, "betas": betas, "params": input_value["params"]}
