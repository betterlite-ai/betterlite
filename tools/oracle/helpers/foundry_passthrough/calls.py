from typing import NotRequired, TypedDict

from pydantic import JsonValue

Params = dict[str, JsonValue]


class RootCall(TypedDict):
    api_base: str


class PrefixCall(TypedDict):
    root: str
    native_endpoint: str


class QueryCall(TypedDict):
    api_base: str
    deployment_api_version: NotRequired[str | None]
    request_query_params: NotRequired[Params | None]


class BaseCall(TypedDict):
    model: str
    endpoint: str
    api_base: NotRequired[str | None]
    global_api_base: NotRequired[str | None]
    environment_api_base: NotRequired[str | None]
    litellm_params: NotRequired[Params | None]
    request_query_params: NotRequired[Params | None]


class ValidationCall(TypedDict, total=False):
    api_key: str | None
    api_base: str | None
    params: Params | None
    headers: dict[str, str]
    environment: dict[str, str]


class OcrCall(TypedDict, total=False):
    response: str
    malformed: bool


class NonStreamCall(TypedDict):
    status: int
    body: str
    model: str
    endpoint: str
    request_url: NotRequired[str | None]
    ocr: NotRequired[OcrCall | None]


class CollectedCall(TypedDict):
    model: str
    endpoint: str
    chunks: NotRequired[list[str]]
    request_data: NotRequired[Params]


class Input(TypedDict, total=False):
    roots: list[RootCall]
    prefixes: list[PrefixCall]
    queries: list[QueryCall]
    bases: list[BaseCall]
    validations: list[ValidationCall]
    nonstreams: list[NonStreamCall]
    streams: list[CollectedCall]
