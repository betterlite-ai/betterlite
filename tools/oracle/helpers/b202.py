from typing import TypedDict

import litellm
from litellm.litellm_core_utils import get_llm_provider_logic as provider_logic


class Input(TypedDict):
    chat_models: list[str]
    text_models: list[str]
    models: list[str]


def run(input_value: Input) -> dict[str, str]:
    chat_models = set(input_value["chat_models"])
    litellm.open_ai_chat_completion_models = chat_models
    litellm.open_ai_text_completion_models = set(input_value["text_models"])
    selected: dict[str, str] = {}
    for model in input_value["models"]:
        resolved, provider, _api_key, _api_base = provider_logic.get_llm_provider(model=model)
        if provider == "openai" and resolved not in chat_models:
            selected[model] = f"text-completion-openai/{resolved}"
        else:
            selected[model] = model
    return selected
