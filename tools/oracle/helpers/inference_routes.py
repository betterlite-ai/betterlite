from typing import TypedDict

from litellm.proxy._types import LiteLLMRoutes


class Input(TypedDict):
    group: str


def run(input_value: Input) -> list[str]:
    group = input_value["group"]
    if group == "openai_routes":
        return LiteLLMRoutes.openai_routes.value
    if group == "anthropic_routes":
        return LiteLLMRoutes.anthropic_routes.value
    if group == "llm_api_routes":
        return LiteLLMRoutes.llm_api_routes.value
    raise ValueError(f"unknown group: {group}")
