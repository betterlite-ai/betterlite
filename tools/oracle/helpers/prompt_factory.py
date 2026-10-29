from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.constants import known_tokenizer_config
from litellm.litellm_core_utils.prompt_templates import factory, huggingface_template_handler

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

custom_prompt = cast(
    Callable[..., str],
    factory.custom_prompt,
)
function_call_prompt = cast(
    Callable[[list[dict[str, Json]], list[Json]], list[dict[str, Json]]],
    factory.function_call_prompt,
)
response_schema_prompt = cast(
    Callable[..., str],
    factory.response_schema_prompt,
)
default_response_schema_prompt = cast(
    Callable[[dict[str, Json]], str],
    factory.default_response_schema_prompt,
)
prompt_factory = cast(
    Callable[..., Json],
    factory.prompt_factory,
)


class InputError(ValueError):
    pass


def _unreachable(hf_model_name: str) -> dict[str, Json]:
    del hf_model_name
    return {"status": "failure"}


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode == "custom_prompt":
        template = cast(dict[str, Json], input_value["template"])
        messages = cast(list[dict[str, Json]], input_value["messages"])
        return custom_prompt(
            role_dict=template["roles"],
            messages=messages,
            initial_prompt_value=template.get("initial_prompt_value", ""),
            final_prompt_value=template.get("final_prompt_value", ""),
            bos_token=template.get("bos_token", ""),
            eos_token=template.get("eos_token", ""),
        )
    if mode == "function_call_prompt":
        messages = cast(list[dict[str, Json]], input_value["messages"])
        functions = cast(list[Json], input_value["functions"])
        return cast(Json, function_call_prompt(messages, functions))
    if mode == "response_schema_prompt":
        response_schema = cast(dict[str, Json], input_value["response_schema"])
        template = cast(dict[str, Json] | None, input_value.get("template"))
        model = "betterlite-oracle-response-schema"
        overrides = {f"{model}/response_schema_prompt": template} if template is not None else {}
        custom_prompt_dict = cast(dict[str, dict[str, Json]], litellm.custom_prompt_dict)
        with patch.dict(custom_prompt_dict, overrides):
            return response_schema_prompt(model=model, response_schema=response_schema)
    if mode == "default_response_schema_prompt":
        response_schema = cast(dict[str, Json], input_value["response_schema"])
        return default_response_schema_prompt(response_schema)
    if mode == "factory":
        model = cast(str, input_value["model"])
        messages = cast(list[dict[str, Json]], input_value["messages"])
        custom_llm_provider = cast(str | None, input_value.get("custom_llm_provider"))
        with (
            patch.dict(cast(dict[str, Json], known_tokenizer_config), {}, clear=True),
            patch.object(huggingface_template_handler, "_get_tokenizer_config", _unreachable),
            patch.object(huggingface_template_handler, "_get_chat_template_file", _unreachable),
        ):
            return prompt_factory(
                model=model, messages=messages, custom_llm_provider=custom_llm_provider
            )
    raise InputError(f"unsupported mode {mode}")
