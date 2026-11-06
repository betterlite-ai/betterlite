from typing import TypeAlias, cast

from litellm.integrations.bitbucket import prompt_initializer
from litellm.integrations.bitbucket.bitbucket_prompt_manager import (
    BitBucketPromptManager,
)
from litellm.types.prompts.init_prompts import PromptLiteLLMParams, PromptSpec

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

CONFIG: dict[str, Json] = {
    "workspace": "ws",
    "repository": "repo",
    "access_token": "token",
    "": "http://127.0.0.1:1/2.0",
}


class InputError(ValueError):
    pass


def manager() -> BitBucketPromptManager:
    params = PromptLiteLLMParams(
        prompt_integration="bitbucket",
        bitbucket_config=CONFIG,  # pyright: ignore[reportCallIssue]
    )
    spec = PromptSpec(prompt_id="case", litellm_params=params)
    return cast(BitBucketPromptManager, prompt_initializer(params, spec))


def load(prompt_channel: BitBucketPromptManager, files: dict[str, str]) -> None:
    templates = prompt_channel.prompt_manager
    for prompt_id, content in files.items():
        template = templates._parse_prompt_file(content, prompt_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        templates.prompts[prompt_id] = template


def parse(prompt_channel: BitBucketPromptManager, input_value: dict[str, Json]) -> Json:
    template = cast(str, input_value["template"])
    messages = prompt_channel._parse_prompt_to_messages(template)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(Json, {"messages": messages})


def compile_prompt(prompt_channel: BitBucketPromptManager, input_value: dict[str, Json]) -> Json:
    prompt_id = cast("str | None", input_value.get("prompt_id"))
    variables = cast("dict[str, Json] | None", input_value.get("variables"))
    client = prompt_channel._compile_prompt_helper(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        prompt_id=prompt_id,
        prompt_spec=None,
        prompt_variables=variables,
        dynamic_callback_params={},
    )
    return cast(
        Json,
        {
            "prompt_id": client["prompt_id"],
            "messages": client["prompt_template"],
            "model": client["prompt_template_model"],
            "optional_params": client["prompt_template_optional_params"],
            "completed_messages": client["completed_messages"],
        },
    )


def get_template(prompt_channel: BitBucketPromptManager, input_value: dict[str, Json]) -> Json:
    prompt_id = cast(str, input_value["prompt_id"])
    variables = cast("dict[str, Json] | None", input_value.get("variables"))
    rendered, metadata = prompt_channel.get_prompt_template(prompt_id, variables)
    return cast(Json, {"rendered": rendered, "metadata": metadata})


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    prompt_channel = manager()
    files = cast("dict[str, str]", input_value.get("files") or {})
    load(prompt_channel, files)
    if mode == "parse":
        return parse(prompt_channel, input_value)
    if mode == "compile":
        return compile_prompt(prompt_channel, input_value)
    if mode == "get_template":
        return get_template(prompt_channel, input_value)
    raise InputError(f"unsupported mode {mode}")
