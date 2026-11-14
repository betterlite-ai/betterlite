import json
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Literal, TypeAlias, TypedDict, cast
from unittest.mock import patch

import litellm
from litellm.llms.github_copilot.chat.transformation import GithubCopilotConfig
from pydantic import JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]

KEY_EXPIRES_AT = 4102444800


class MessagesInput(TypedDict):
    action: Literal["transform_messages"]
    disable_system_to_assistant: bool
    model: str
    messages: list[JsonObject]


class EnvironmentInput(TypedDict):
    action: Literal["validate_environment"]
    api_key: str | None
    copilot_key: str
    headers: dict[str, str]
    model: str
    messages: list[JsonObject]


class ParamsInput(TypedDict):
    action: Literal["supported_params"]
    model: str
    # betterlite answers supports_reasoning from this list, LiteLLM from its packaged price map
    reasoning_models: list[str]


class ReplyInput(TypedDict):
    action: Literal["repair_reply"]
    response: JsonObject


Input: TypeAlias = MessagesInput | EnvironmentInput | ParamsInput | ReplyInput


def transform_messages(step: MessagesInput) -> JsonValue:
    previous = litellm.disable_copilot_system_to_assistant
    litellm.disable_copilot_system_to_assistant = step["disable_system_to_assistant"]
    try:
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L60
            JsonValue,
            GithubCopilotConfig()._transform_messages(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L60
                step["messages"], step["model"]
            ),
        )
    finally:
        litellm.disable_copilot_system_to_assistant = previous


def validate_environment(step: EnvironmentInput) -> JsonValue:
    with TemporaryDirectory() as root:
        key_file = {"token": step["copilot_key"], "expires_at": KEY_EXPIRES_AT}
        _ = (Path(root) / "api-key.json").write_text(json.dumps(key_file))
        with patch.dict("os.environ", {"GITHUB_COPILOT_TOKEN_DIR": root}):
            config = GithubCopilotConfig()
            headers = config.validate_environment(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L86
                headers=step["headers"],
                model=step["model"],
                messages=step["messages"],  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                optional_params={},
                litellm_params={},
                api_key=step["api_key"],
            )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L86
        JsonValue, headers
    )


def supported_params(step: ParamsInput) -> JsonValue:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L119
        JsonValue, GithubCopilotConfig().get_supported_openai_params(step["model"])
    )


def repair_reply(step: ReplyInput) -> JsonValue:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L269
        JsonValue, GithubCopilotConfig().transform_parsed_response_dict(step["response"])
    )


def run(input_value: Input) -> JsonValue:
    if input_value["action"] == "transform_messages":
        return transform_messages(input_value)
    if input_value["action"] == "validate_environment":
        return validate_environment(input_value)
    if input_value["action"] == "supported_params":
        return supported_params(input_value)
    return repair_reply(input_value)
