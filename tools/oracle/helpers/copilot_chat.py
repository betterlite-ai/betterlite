import json
import os
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Literal, TypeAlias, TypedDict, cast

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
    config = GithubCopilotConfig()
    optional_params: JsonObject = {}
    litellm_params: JsonObject = {}
    request_headers: dict[str, str] = {}
    try:
        request = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L261
            JsonObject,
            config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L261
                model=step["model"],
                messages=step["messages"],  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                optional_params=optional_params,
                litellm_params=litellm_params,
                headers=request_headers,
            ),
        )
    finally:
        litellm.disable_copilot_system_to_assistant = previous
    return request["messages"]


def validate_environment(step: EnvironmentInput) -> JsonValue:
    with TemporaryDirectory() as root:
        key_file = {"token": step["copilot_key"], "expires_at": KEY_EXPIRES_AT}
        _ = (Path(root) / "api-key.json").write_text(json.dumps(key_file))
        previous = os.environ.get("GITHUB_COPILOT_TOKEN_DIR")
        os.environ["GITHUB_COPILOT_TOKEN_DIR"] = root
        optional_params: JsonObject = {}
        litellm_params: JsonObject = {}
        try:
            config = GithubCopilotConfig()
            headers = config.validate_environment(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L86
                headers=step["headers"],
                model=step["model"],
                messages=step["messages"],  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                optional_params=optional_params,
                litellm_params=litellm_params,
                api_key=step["api_key"],
            )
        finally:
            if previous is None:
                os.environ.pop("GITHUB_COPILOT_TOKEN_DIR", None)
            else:
                os.environ["GITHUB_COPILOT_TOKEN_DIR"] = previous
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L86
        JsonValue, headers
    )


def supported_params(step: ParamsInput) -> JsonValue:
    return cast(JsonValue, GithubCopilotConfig().get_supported_openai_params(step["model"]))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L119


def repair_reply(step: ReplyInput) -> JsonValue:
    return cast(JsonValue, GithubCopilotConfig().transform_parsed_response_dict(step["response"]))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github_copilot/chat/transformation.py#L269


def run(input_value: Input) -> JsonValue:
    if input_value["action"] == "transform_messages":
        return transform_messages(input_value)
    if input_value["action"] == "validate_environment":
        return validate_environment(input_value)
    if input_value["action"] == "supported_params":
        return supported_params(input_value)
    return repair_reply(input_value)
