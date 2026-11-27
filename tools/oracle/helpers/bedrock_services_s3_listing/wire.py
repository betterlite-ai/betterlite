import json
import os
import time
from datetime import UTC, datetime
from types import MappingProxyType
from typing import Final, Literal, NotRequired, TypedDict, cast

import httpx
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
SIGNING_SECONDS: Final = 1_704_067_200
ENV_ACCESS_KEY: Final = "AKIDEXAMPLE"
ENV_KEY_VALUE: Final = "secret"
TRUSTED_KEY: Final = "_litellm_internal_model_credentials"
BASE_ENVIRONMENT: Final[dict[str, str]] = {
    "LITELLM_RUST": "false",
    "AWS_CONFIG_FILE": os.devnull,
    "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
    "AWS_ACCESS_KEY_ID": ENV_ACCESS_KEY,
    "AWS_SECRET_ACCESS_KEY": ENV_KEY_VALUE,
    "TZ": "UTC",
}


class ListCall(TypedDict):
    purpose: NotRequired[str | None]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class NextCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class ListingCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class FileCall(TypedDict):
    file_id: NotRequired[str | None]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class DeleteResponseCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    additional_args: NotRequired[dict[str, JsonValue] | None]


class RetrieveCall(TypedDict):
    file_id: str
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]


class ListRequests(TypedDict):
    operation: Literal["list_request"]
    requests: list[ListCall]


class NextRequests(TypedDict):
    operation: Literal["next_request"]
    next: list[NextCall]


class ListResponses(TypedDict):
    operation: Literal["list_response"]
    listings: list[ListingCall]


class DeleteRequests(TypedDict):
    operation: Literal["delete_request"]
    deletes: list[FileCall]


class ContentRequests(TypedDict):
    operation: Literal["content_request"]
    contents: list[FileCall]


class DeleteResponses(TypedDict):
    operation: Literal["delete_response"]
    delete_responses: list[DeleteResponseCall]


class Retrieves(TypedDict):
    operation: Literal["retrieve"]
    retrieves: list[RetrieveCall]


Input = (
    ListRequests
    | NextRequests
    | ListResponses
    | DeleteRequests
    | ContentRequests
    | DeleteResponses
    | Retrieves
)


class FixedNow:
    def __init__(self, seconds: int) -> None:
        self.moment = datetime.fromtimestamp(seconds, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


def reset_environment(environment: dict[str, str] | None) -> None:
    os.environ.clear()
    os.environ.update(BASE_ENVIRONMENT)
    if environment is not None:
        os.environ.update(environment)
    time.tzset()


def params_of(
    declared: dict[str, JsonValue] | None, trusted: list[tuple[str, JsonValue]] | None
) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {} if declared is None else dict(declared)
    if trusted is not None:
        params[TRUSTED_KEY] = cast(JsonValue, MappingProxyType(dict(trusted)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L192-L198
    return params


def optional_of(declared: dict[str, JsonValue] | None) -> dict[str, JsonValue]:
    if declared is None:
        return {}
    return dict(declared)


def visible(params: dict[str, JsonValue]) -> JsonValue:
    without_trusted = {name: value for name, value in params.items() if name != TRUSTED_KEY}
    return JSON.validate_json(json.dumps(without_trusted, default=dict))


def reply(answer: JsonValue) -> JsonValue:
    encoded: Final = json.dumps(answer)
    return JSON.validate_json(encoded)


def response_of(
    status: int, body: str, headers: list[tuple[str, str]] | None
) -> httpx.Response:
    pairs: list[tuple[str, str]] = [] if headers is None else headers
    return httpx.Response(status_code=status, headers=pairs, content=body.encode())
