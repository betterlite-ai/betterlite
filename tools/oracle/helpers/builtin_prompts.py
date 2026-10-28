from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils.prompt_templates import factory

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Renderer: TypeAlias = Callable[[list[dict[str, Json]]], Json]


class InputError(ValueError):
    pass


def injected_template(model: str, messages: list[dict[str, Json]]) -> str:
    return model + ":" + " ".join(cast(str, message["content"]) for message in messages)


def run(input_value: dict[str, Json]) -> Json:
    family = cast(str, input_value["family"])
    messages = cast(list[dict[str, Json]], input_value["messages"])
    names = {
        "default": "default_pt", "system_map": "map_system_message_pt",
        "alpaca": "alpaca_pt", "llama2": "llama_2_chat_pt",
        "mistral": "mistral_instruct_pt", "falcon_instruct": "falcon_instruct_pt",
        "falcon_chat": "falcon_chat_pt", "mpt": "mpt_chat_pt",
        "wizardcoder": "wizardcoder_pt", "phind": "phind_codellama_pt",
        "deepseek": "deepseek_r1_pt", "claude": "anthropic_pt",
        "claude21": "claude_2_1_pt", "granite": "ibm_granite_pt", "titan": "amazon_titan_pt",
    }
    if family == "ollama_image":
        convert = cast(Callable[[str], str], factory.convert_to_ollama_image)
        return convert(cast(str, input_value["model"]))
    if family == "ollama":
        render_ollama = cast(Callable[[str, list[dict[str, Json]]], Json], factory.ollama_pt)
        return render_ollama(cast(str, input_value.get("model", "")), messages)
    if family not in names:
        raise InputError(f"unsupported family {family}")
    render = cast(Renderer, getattr(factory, names[family]))
    with patch.object(factory, "hf_chat_template", injected_template):
        return render(messages)
