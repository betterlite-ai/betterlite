import os
from typing import TypedDict

import litellm
from pydantic import JsonValue

Object = dict[str, JsonValue]

MODEL = "m"
MANAGED_ENV = ("EDENAI_API_KEY", "EDENAI_API_BASE")


class Call(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    global_api_key: str | None
    api_base: str | None
    environment: dict[str, str]


class ChatParams(TypedDict):
    model: str
    reasoning: bool


class ChatRequest(TypedDict):
    model: str
    optional_params: Object


class CatalogCase(TypedDict):
    status: int
    text: str
    headers: dict[str, str]
    api_base: str | None
    environment: dict[str, str]


class EmbeddingPlan(TypedDict):
    model: str
    input: JsonValue
    optional_params: Object
    non_default_params: Object
    headers: dict[str, str]
    api_key: str | None
    global_api_key: str | None
    api_base: str | None
    environment: dict[str, str]


class Input(TypedDict):
    costs: list[JsonValue]
    cost_bodies: list[str]
    cost_headers: list[dict[str, str]]
    credentials: list[Call]
    chat_params: list[ChatParams]
    chat_requests: list[ChatRequest]
    chat_replies: list[str]
    chunks: list[Object]
    models: list[CatalogCase]
    embedding_plans: list[EmbeddingPlan]
    embedding_replies: list[str]
    responses_prepare: list[Call]
    responses_replies: list[str]
    messages_environments: list[Call]
    messages_replies: list[str]


def apply_environment(environment: dict[str, str], global_api_key: str | None) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value
    litellm.api_key = global_api_key
