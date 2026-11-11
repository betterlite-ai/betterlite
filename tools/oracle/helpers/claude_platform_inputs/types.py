import logging
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import NotRequired, TypeAlias, TypedDict

from litellm._logging import verbose_logger
from litellm.exceptions import AuthenticationError
from litellm.types.llms.bedrock import AwsAuthParams
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Override: TypeAlias = Json | set[Json] | frozenset[Json] | tuple[Json, ...]

EMPTY_PARAMS: dict[str, Json] = {}
EMPTY_STRINGS: dict[str, str] = {}
EMPTY_HEADERS: list[tuple[str, str]] = []
EMPTY_HEADER_MAP: dict[str, str | None] = {}
OVERRIDE_KEY = "claude_platform_unsupported_params"
DROP_MARKER = "dropping unsupported Messages API param(s) "
IGNORE_MARKER = "ignoring claude_platform_unsupported_params of type "


class WorkspaceCase(TypedDict):
    optional_params: NotRequired[dict[str, Json]]
    deployment: NotRequired[dict[str, str]]
    litellm_settings: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]


class RegionCase(TypedDict):
    optional_params: NotRequired[dict[str, Json]]
    environment: NotRequired[dict[str, str]]


class UrlCase(TypedDict):
    api_base: NotRequired[str | None]
    litellm_api_base: NotRequired[str | None]
    optional_params: NotRequired[dict[str, Json]]
    environment: NotRequired[dict[str, str]]


class OverrideCall(TypedDict):
    deployment: NotRequired[dict[str, Json]]
    optional: NotRequired[dict[str, Json]]
    log_invalid: NotRequired[bool]


class BodyCall(TypedDict):
    params: NotRequired[dict[str, Json]]
    unsupported: NotRequired[list[str]]
    log_dropped: NotRequired[bool]


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class SignCase(TypedDict):
    headers: NotRequired[dict[str, str | None]]
    optional_params: NotRequired[dict[str, Json]]
    request_data: NotRequired[dict[str, Json]]
    api_base: str
    api_key: NotRequired[str | None]
    model: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]
    credentials: Keys
    now: int


class ErrorClassCall(TypedDict):
    message: str
    status_code: int
    headers: NotRequired[list[tuple[str, str]]]


class SigningCall(TypedDict):
    sign: NotRequired[SignCase]
    error_class: NotRequired[ErrorClassCall]


class Input(TypedDict):
    workspace_alias_precedence: NotRequired[dict[str, list[WorkspaceCase]]]
    explicit_region_required: NotRequired[dict[str, list[RegionCase]]]
    api_base_before_region: NotRequired[dict[str, list[UrlCase]]]
    platform_route_prefix: NotRequired[dict[str, list[str]]]
    unsupported_override: NotRequired[dict[str, list[OverrideCall]]]
    filtered_body_fields: NotRequired[dict[str, list[BodyCall]]]
    platform_signing: NotRequired[dict[str, list[SigningCall]]]


class FrozenClock:
    def __init__(self, epoch: int) -> None:
        self.moment = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


class FixedCredentials:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token = keys.get("session_token")


class CredentialProvider:
    def __init__(self, keys: Keys) -> None:
        self.credentials = FixedCredentials(keys)

    def __call__(self, _auth: AwsAuthParams, _region: str | None) -> FixedCredentials:
        return self.credentials


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.entries: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.entries.append(warning_entry(record.getMessage()))


def set_environment(environment: dict[str, str] | None) -> None:
    os.environ.clear()
    os.environ["LITELLM_RUST"] = "false"
    if environment is not None:
        os.environ.update(environment)


@contextmanager
def captured_warnings() -> Generator[list[Json], None, None]:
    handler = Warnings()
    verbose_logger.addHandler(handler)
    try:
        yield handler.entries
    finally:
        verbose_logger.removeHandler(handler)


def warning_entry(message: str) -> Json:
    if DROP_MARKER in message:
        _, _, listed = message.partition(DROP_MARKER)
        tuple_text, _, _ = listed.partition(" from the request body")
        drop: Json = {"action": "drop", "params": parse_names(tuple_text)}
        return drop
    _, _, type_text = message.partition(IGNORE_MARKER)
    ignore: Json = {"action": "ignore", "type_name": type_text.split(";", 1)[0]}
    return ignore


def parse_names(text: str) -> list[Json]:
    inner = text.strip()[1:-1].rstrip(",")
    if not inner:
        return []
    return [name.strip("'\"") for name in inner.split(", ")]


def override_container(spec: dict[str, Json]) -> Override:
    kind, payload = next(iter(spec.items()))
    if kind == "other":
        return payload
    match payload:
        case [*items]:
            listed: list[Json] = items
        case _:
            raise ValueError("override items must be a list")
    if kind == "set":
        return set(listed)
    if kind == "frozenset":
        return frozenset(listed)
    if kind == "tuple":
        return tuple(listed)
    return listed


def resolution_outcome(key: str, resolve: Callable[[], str]) -> Json:
    try:
        value = resolve()
    except AuthenticationError:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L149
        failure: Json = {"error": "missing_region"}
        return failure
    except ValueError:
        invalid: Json = {"error": "invalid_region"}
        return invalid
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L106
        other: Json = {"error": "other"}
        return other
    else:
        success: Json = {key: value}
        return success
