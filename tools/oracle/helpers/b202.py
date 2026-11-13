from typing import TypedDict

import litellm


class Input(TypedDict):
    models: list[str]


def run(input_value: Input) -> dict[str, str]:
    selected: dict[str, str] = {}
    for model in input_value["models"]:
        resolved, provider, _api_key, _api_base = litellm.get_llm_provider(model=model)
        if provider == "openai" and resolved not in litellm.open_ai_chat_completion_models:
            selected[model] = f"text-completion-openai/{resolved}"
        else:
            selected[model] = f"{provider}/{resolved}"
    return selected
