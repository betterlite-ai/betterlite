from typing import Literal, TypedDict

from pydantic import JsonValue

Action = Literal[
    "messages", "legacy", "count", "batch_create", "batch_retrieve", "batch_results",
    "upload", "retrieve", "list", "delete", "content", "config_url",
]


class Input(TypedDict):
    action: Action
    model: str
    provider: str
    api_base: str | None
    api_key: str | None
    global_base: str | None
    anthropic_key: str | None
    global_key: str | None
    environment: dict[str, str]
    disable_suffix: bool
    config_present: bool
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]
    custom_prompt_dict: dict[str, JsonValue]
    global_prompt_dict: dict[str, JsonValue]
    stream: bool
    acompletion: bool
    client: str | None
    shared_session: str | None
    encoding: str
    expected_key: str | None
    failure: int | None
    response: str
    id: str
    purpose: str | None
    file: str
    system: JsonValue
    tools: list[dict[str, JsonValue]] | None


class Recorded(TypedDict):
    method: str
    url: str
    query: list[list[str]]
    headers: dict[str, str]
    body: JsonValue


class Observation(TypedDict):
    result: JsonValue
    post_call: bool
    post_matches: bool
    request: Recorded | None


class Case(TypedDict):
    inputs: list[Input]
    expected: list[Observation]
