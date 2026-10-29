from typing import TypeAlias, cast

from jinja2.sandbox import ImmutableSandboxedEnvironment
from litellm.litellm_core_utils.prompt_templates.factory import (
    _render_chat_template,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)
from litellm.litellm_core_utils.prompt_templates.huggingface_template_handler import (
    _extract_token_value,  # pyright: ignore[reportPrivateUsage]
    strftime_now,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
TokenValue: TypeAlias = None | str | dict[str, Json]


class InputError(ValueError):
    pass


def build_env() -> ImmutableSandboxedEnvironment:
    env = ImmutableSandboxedEnvironment()
    env.globals["raise_exception"] = lambda msg: Exception(f"Error message - {msg}")  # pyright: ignore[reportArgumentType]
    env.globals["strftime_now"] = strftime_now  # pyright: ignore[reportArgumentType]
    return env


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode == "extract_token":
        token = cast(TokenValue, input_value.get("token"))
        return _extract_token_value(token_value=token)
    if mode != "render":
        raise InputError(f"unsupported mode {mode}")
    chat_template = cast(str, input_value["chat_template"])
    bos_token = _extract_token_value(token_value=cast(TokenValue, input_value.get("bos_token")))
    eos_token = _extract_token_value(token_value=cast(TokenValue, input_value.get("eos_token")))
    messages = cast(list[dict[str, Json]], input_value["messages"])
    return _render_chat_template(
        env=build_env(),
        chat_template=chat_template,
        bos_token=bos_token,
        eos_token=eos_token,
        messages=messages,
    )
