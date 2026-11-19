import json
import os
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, Unpack, runtime_checkable

import litellm
from litellm.llms.github_copilot.embedding.transformation import (
    GithubCopilotEmbeddingConfig,
)
from litellm.llms.github_copilot.messages.transformation import (
    GithubCopilotAnthropicMessagesConfig,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

EXPIRES_AT: Final = 4102444800
RECORD_FILE: Final = "api-key.json"
DIRECTORY_VARIABLE: Final = "GITHUB_COPILOT_TOKEN_DIR"
API_BASE_VARIABLE: Final = "GITHUB_COPILOT_API_BASE"
NO_PARAMS: Final[dict[str, JsonValue]] = {}
NO_MESSAGES: Final[list[JsonValue]] = []
HEADERS: Final = TypeAdapter(dict[str, JsonValue])

EmbeddingValues: TypeAlias = str | list[str] | list[int] | list[list[int]]


class EmbeddingCase(TypedDict):
    action: Literal["embedding"]
    model: str
    input: EmbeddingValues
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    headers: dict[str, str]
    # an absent token and a False-like one both reach LiteLLM's AuthenticationError
    copilot_key: str | None
    api_base: str | None
    cached_base: str | None
    env_base: str | None


class MessagesCase(TypedDict):
    action: Literal["messages"]
    model: str
    headers: dict[str, str]
    optional_params: dict[str, JsonValue]
    messages: list[JsonValue]
    copilot_key: str | None
    cached_base: str | None
    # LiteLLM discards the untrusted caller base it is handed
    caller_api_base: str | None


Case: TypeAlias = EmbeddingCase | MessagesCase


class EnvironmentArgs(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    api_key: str | None
    api_base: str | None


class UrlArgs(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    stream: bool | None


@runtime_checkable
class EmbeddingConfig(Protocol):
    def validate_environment(
        self, **kwargs: Unpack[EnvironmentArgs]
    ) -> dict[str, str]: ...

    def transform_embedding_request(
        self,
        model: str,
        input_values: EmbeddingValues,
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(self, **kwargs: Unpack[UrlArgs]) -> str: ...

    def map_openai_params(
        self,
        *,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


@runtime_checkable
class MessagesConfig(Protocol):
    def validate_anthropic_messages_environment(  # ast-grep-ignore: no-long-names
        self, **kwargs: Unpack[EnvironmentArgs]
    ) -> tuple[dict[str, str], str | None]: ...

    def get_complete_url(self, **kwargs: Unpack[UrlArgs]) -> str: ...


class EmbeddingBound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: EmbeddingConfig


class MessagesBound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: MessagesConfig


def embedding_config() -> EmbeddingConfig:
    holder: dict[str, GithubCopilotEmbeddingConfig] = {
        "config": GithubCopilotEmbeddingConfig()
    }
    return EmbeddingBound.model_validate(holder).config


def messages_config() -> MessagesConfig:
    holder = {"config": GithubCopilotAnthropicMessagesConfig()}
    return MessagesBound.model_validate(holder).config


def key_record(copilot_key: str | None, cached_base: str | None) -> str:
    record: dict[str, JsonValue] = {"expires_at": EXPIRES_AT}
    if copilot_key is not None:
        record["token"] = copilot_key
    if cached_base is not None:
        endpoints: dict[str, JsonValue] = {"api": cached_base}
        record["endpoints"] = endpoints
    return json.dumps(record)


def restore(previous: str | None, name: str) -> None:
    if previous is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = previous


def embedding_answer(root: str, case: EmbeddingCase) -> JsonValue:
    _ = (Path(root) / RECORD_FILE).write_text(
        key_record(case["copilot_key"], case["cached_base"])
    )
    config = embedding_config()
    headers = config.validate_environment(
        headers=dict(case["headers"]),
        model=case["model"],
        messages=NO_MESSAGES,
        optional_params=NO_PARAMS,
        litellm_params=NO_PARAMS,
        api_key=None,
        api_base=None,
    )
    optional: dict[str, JsonValue] = dict(case["optional_params"])
    _ = config.map_openai_params(
        non_default_params=dict(case["non_default_params"]),
        optional_params=optional,
        model=case["model"],
        drop_params=True,
    )
    body = config.transform_embedding_request(
        case["model"], case["input"], dict(optional), headers
    )
    url = config.get_complete_url(
        api_base=case["api_base"],
        api_key=None,
        model=case["model"],
        optional_params=NO_PARAMS,
        litellm_params=NO_PARAMS,
        stream=None,
    )
    return {"headers": HEADERS.validate_python(headers), "url": url, "body": body}


def messages_answer(root: str, case: MessagesCase) -> JsonValue:
    _ = (Path(root) / RECORD_FILE).write_text(
        key_record(case["copilot_key"], case["cached_base"])
    )
    config = messages_config()
    optional: dict[str, JsonValue] = dict(case["optional_params"])
    headers, api_base = config.validate_anthropic_messages_environment(
        headers=dict(case["headers"]),
        model=case["model"],
        messages=case["messages"],
        optional_params=optional,
        litellm_params=NO_PARAMS,
        api_key=None,
        api_base=case["caller_api_base"],
    )
    url = config.get_complete_url(
        api_base=api_base,
        api_key=None,
        model=case["model"],
        optional_params=NO_PARAMS,
        litellm_params=NO_PARAMS,
        stream=None,
    )
    return {
        "headers": HEADERS.validate_python(headers),
        "api_base": api_base,
        "url": url,
    }


def embedding(case: EmbeddingCase) -> JsonValue:
    with TemporaryDirectory() as root:
        previous_dir = os.environ.get(DIRECTORY_VARIABLE)
        previous_base = os.environ.get(API_BASE_VARIABLE)
        os.environ[DIRECTORY_VARIABLE] = root
        if case["env_base"] is not None:
            os.environ[API_BASE_VARIABLE] = case["env_base"]
        try:
            return embedding_answer(root, case)
        finally:
            restore(previous_dir, DIRECTORY_VARIABLE)
            restore(previous_base, API_BASE_VARIABLE)


def messages(case: MessagesCase) -> JsonValue:
    with TemporaryDirectory() as root:
        previous = os.environ.get(DIRECTORY_VARIABLE)
        os.environ[DIRECTORY_VARIABLE] = root
        try:
            return messages_answer(root, case)
        finally:
            restore(previous, DIRECTORY_VARIABLE)


def run(input_value: Case) -> JsonValue:
    litellm.suppress_debug_info = True
    match input_value["action"]:
        case "embedding":
            return embedding(input_value)
        case "messages":
            return messages(input_value)
