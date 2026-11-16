import os
from typing import TypedDict

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True

Object = dict[str, JsonValue]

MODEL = "accounts/fireworks/models/test-model"
# the Rust helper fixes the uuid4 and the clock the same way, and a case compares both sides
FIXED_UUID = "00000000-0000-0000-0000-000000000001"
FIXED_TIME = 1_700_000_000
MANAGED_ENV = (
    "FIREWORKS_API_KEY",
    "FIREWORKS_AI_API_KEY",
    "FIREWORKSAI_API_KEY",
    "FIREWORKS_AI_TOKEN",
    "FIREWORKS_API_BASE",
    "FIREWORKS_ACCOUNT_ID",
)


class CatalogModel(TypedDict):
    key: str
    model_info: Object


class HeaderCall(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    litellm_params: Object
    environment: dict[str, str]


class KeyCall(TypedDict):
    api_key: str | None
    environment: dict[str, str]


class RequestCall(TypedDict):
    model: str
    messages: list[Object]
    optional_params: Object


class ReplyCall(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    optional_params: Object


class ChunkCall(TypedDict):
    chunk: Object


class ModelCall(TypedDict):
    api_base: str | None
    api_key: str | None
    environment: dict[str, str]
    status: int
    body: str


class Input(TypedDict):
    catalog: list[CatalogModel]
    headers: list[HeaderCall]
    keys: list[KeyCall]
    requests: list[RequestCall]
    replies: list[ReplyCall]
    chunks: list[ChunkCall]
    models: list[ModelCall]


def activate(environment: dict[str, str]) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value
