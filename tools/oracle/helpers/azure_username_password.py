import os
import time
import warnings
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Self, TypeAlias, TypedDict, cast
from unittest import mock

import azure.identity
from azure.core.credentials import TokenProvider
from azure.core.exceptions import ClientAuthenticationError
from azure.identity import UsernamePasswordCredential
from litellm._logging import verbose_logger
from litellm.llms.azure.common_utils import get_azure_ad_token_from_username_password

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
DEPRECATION_PREFIX = "Failed to build AuthenticationRecord from unexpected identity token"
RESERVED_PREFIX = "Authentication failed: You cannot use any scope value that is reserved."
HTTP_ERROR_STATUS = 400
NOW = 1_700_000_000
CREDENTIAL_ARGUMENTS = {"client_id", "password", "username"}


class ReplyRequired(TypedDict):
    status: int
    body: str


class Reply(ReplyRequired, total=False):
    url_contains: str
    error: str


class Step(TypedDict, total=False):
    environment: dict[str, str]
    authority_host: str | None
    client_id: str
    username: str
    password: str
    scope: str | None
    replies: list[Reply]
    calls: int
    advance: int
    observe: str


class Input(TypedDict):
    action: str
    steps: list[Step]


class RecordedRequest(TypedDict):
    method: str
    url: str
    query: dict[str, str]
    form: dict[str, str]


class FixtureResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.text = reply["body"]
        self.headers: dict[str, str] = {}

    def raise_for_status(self) -> None:
        if self.status_code >= HTTP_ERROR_STATUS:
            raise RuntimeError(f"HTTP Error: {self.status_code}")


class FixtureClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[RecordedRequest] = []
        self.failed = False

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def close(self) -> None:
        return None

    def get_error_response(self, _result: object) -> None:
        return None

    def _record(
        self,
        method: str,
        url: str,
        params: dict[str, object] | None,
        data: dict[str, object] | None,
    ) -> None:
        self.requests.append(
            {
                "method": method,
                "url": url,
                "query": {name: str(value) for name, value in (params or {}).items()},
                "form": {name: str(value) for name, value in (data or {}).items()},
            }
        )

    def _reply(self, url: str) -> FixtureResponse:
        for index, reply in enumerate(self.replies):
            needle = reply.get("url_contains")
            if needle is None or needle in url:
                matched = self.replies.pop(index)
                error = matched.get("error")
                if error is not None:
                    self.failed = True
                    raise OSError(error)
                return FixtureResponse(matched)
        raise AssertionError(f"no reply for {url}")

    def get(
        self,
        url: str,
        params: dict[str, object] | None = None,
        _headers: dict[str, str] | None = None,
        **_kwargs: object,
    ) -> FixtureResponse:
        self._record("GET", url, params, None)
        return self._reply(url)

    def post(
        self,
        url: str,
        params: dict[str, object] | None = None,
        data: dict[str, object] | None = None,
        _headers: dict[str, str] | None = None,
        **_kwargs: object,
    ) -> FixtureResponse:
        self._record("POST", url, params, data)
        return self._reply(url)


class Live:
    def __init__(
        self,
        client: FixtureClient,
        constructor: mock.MagicMock,
        debug: list[tuple[object, tuple[object, ...]]],
        clock: mock.MagicMock,
        caught: list[warnings.WarningMessage],
    ) -> None:
        self.client = client
        self.constructor = constructor
        self.debug = debug
        self.clock = clock
        self.caught = caught

    @property
    def credentials(self) -> list[UsernamePasswordCredential]:
        return CREDENTIALS

    @property
    def scopes(self) -> list[tuple[str, ...]]:
        return SCOPES

    @property
    def constructor_kwargs(self) -> set[str]:
        call = self.constructor.call_args
        if call is None:
            return set()
        return set(cast("dict[str, object]", call.kwargs))

    def advance_to(self, advance: int) -> None:
        self.clock.return_value = float(NOW + advance)


CREDENTIALS: list[UsernamePasswordCredential] = []
SCOPES: list[tuple[str, ...]] = []
DEBUG_CALLS: list[tuple[object, tuple[object, ...]]] = []
REAL_BEARER_TOKEN_PROVIDER = azure.identity.get_bearer_token_provider


def build_credential(client_id: str, username: str, password: str) -> UsernamePasswordCredential:
    credential = UsernamePasswordCredential(client_id, username, password)
    CREDENTIALS.append(credential)
    return credential


def recording_bearer_provider(
    credential: TokenProvider, *scopes: str
) -> Callable[[], str]:
    SCOPES.append(scopes)
    return REAL_BEARER_TOKEN_PROVIDER(credential, *scopes)


@contextmanager
def fixture_environment(environment: dict[str, str]) -> Generator[None]:
    with mock.patch.dict(os.environ, environment, clear=True):
        yield


@contextmanager
def fixed_clock() -> Generator[mock.MagicMock]:
    with mock.patch.object(time, "time", return_value=float(NOW)) as clock:
        yield clock


def capture_debug(message: object, *args: object) -> None:
    DEBUG_CALLS.append((message, args))


@contextmanager
def session(step: Step) -> Generator[Live]:
    client = FixtureClient(list(step.get("replies", [])))
    CREDENTIALS.clear()
    SCOPES.clear()
    DEBUG_CALLS.clear()
    constructor = mock.MagicMock(side_effect=build_credential)
    with (
        fixture_environment(environment_of(step)),
        mock.patch(
            "azure.identity._internal.msal_credentials.MsalClient",
            return_value=client,
        ),
        fixed_clock() as clock,
        mock.patch.object(azure.identity, "UsernamePasswordCredential", constructor),
        mock.patch.object(
            azure.identity, "get_bearer_token_provider", recording_bearer_provider
        ),
        mock.patch.object(verbose_logger, "debug", capture_debug),
        warnings.catch_warnings(record=True) as caught,
    ):
        warnings.simplefilter("always")
        yield Live(client, constructor, DEBUG_CALLS, clock, caught)


def environment_of(step: Step) -> dict[str, str]:
    environment = dict(step.get("environment", {}))
    host = step.get("authority_host")
    if host is not None:
        environment["AZURE_AUTHORITY_HOST"] = host
    return environment


def scope_of(step: Step) -> str:
    scope = step.get("scope")
    return DEFAULT_SCOPE if scope is None else scope


def build_provider(step: Step) -> Callable[[], str]:
    client_id = step.get("client_id", "")
    username = step.get("username", "")
    password = step.get("password", "")
    scope = step.get("scope")
    if scope is None:
        return get_azure_ad_token_from_username_password(client_id, username, password)
    return get_azure_ad_token_from_username_password(
        client_id, username, password, scope
    )


def posts_of(client: FixtureClient) -> list[RecordedRequest]:
    return [request for request in client.requests if request["method"] == "POST"]


def discovery_of(client: FixtureClient) -> str | None:
    for request in client.requests:
        if "well-known/openid-configuration" in request["url"]:
            return request["url"]
    return None


def first_form(client: FixtureClient) -> dict[str, str] | None:
    for request in posts_of(client):
        return request["form"]
    return None


def first_post_url(client: FixtureClient) -> str | None:
    for request in posts_of(client):
        return request["url"]
    return None


def record_of(credential: UsernamePasswordCredential | None) -> Json:
    if credential is None:
        return None
    record = credential._auth_record  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    if record is None:
        return None
    return {
        "authority": record.authority,
        "client_id": record.client_id,
        "home_account_id": record.home_account_id,
        "tenant_id": record.tenant_id,
        "username": record.username,
    }


def error_json(reason: str, message: str, client: FixtureClient) -> dict[str, Json]:
    return {
        "kind": "error",
        "reason": reason,
        "message": message,
        "token_requests": len(posts_of(client)),
    }


def authentication_detail(text: str, posts: int) -> tuple[str, str]:
    if text.startswith(DEPRECATION_PREFIX):
        return "auth_record", text
    if text.startswith(RESERVED_PREFIX):
        return "reserved_scope", text.split("\n", maxsplit=1)[0]
    return "authentication_failed", text if posts > 0 else ""


def key_detail(error: KeyError) -> tuple[str, str]:
    if next(iter(error.args), None) == "expires_in":
        return "missing_expires_in", ""
    return "other", ""


def failure_detail(error: Exception, posts: int) -> tuple[str, str]:
    if isinstance(error, ClientAuthenticationError):
        return authentication_detail(str(error), posts)
    if isinstance(error, KeyError):
        return key_detail(error)
    if isinstance(error, ValueError):
        return "constructor", str(error)
    return "other", ""


def failure_of(error: Exception, live: Live) -> dict[str, Json]:
    posts = len(posts_of(live.client))
    reason, message = failure_detail(error, posts)
    if live.client.failed:
        reason, message = "transport", ""
    return error_json(reason, message, live.client)


def grant_json(token: str, live: Live) -> Json:
    form = first_form(live.client)
    if form is None:
        return error_json("no_token_request", "", live.client)
    return cast(
        "Json",
        {
            "kind": "token",
            "credential_arguments": sorted(live.constructor_kwargs & CREDENTIAL_ARGUMENTS),
            "scope_arguments": list(next(iter(live.scopes), ())),
            "discovery": discovery_of(live.client),
            "token_url": first_post_url(live.client),
            "form": {name: value for name, value in form.items() if name != "scope"},
            "scope": sorted(form.get("scope", "").split()),
            "token": token,
        },
    )


def call_provider(provider: Callable[[], str], live: Live, step: Step) -> list[str]:
    calls = int(step.get("calls", 1))
    advance = int(step.get("advance", 0))
    tokens: list[str] = []
    for index in range(calls):
        if index > 0:
            live.advance_to(advance)
        tokens.append(provider())
    return tokens


def grant_result(step: Step) -> Json:
    with session(step) as live:
        try:
            provider = build_provider(step)
            token = provider()
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            return failure_of(error, live)
        return grant_json(token, live)


def attempt_result(step: Step) -> Json:
    with session(step) as live:
        try:
            provider = build_provider(step)
            tokens = call_provider(provider, live, step)
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            return failure_of(error, live)
        credential = CREDENTIALS[0] if CREDENTIALS else None
        return cast(
            "Json",
            {
                "kind": "token",
                "tokens": tokens,
                "token_requests": len(posts_of(live.client)),
                "record": record_of(credential),
            },
        )


def info_result(step: Step) -> Json:
    with session(step) as live:
        try:
            build_provider(step)
            credential = CREDENTIALS[0]
            info = credential.get_token_info(scope_of(step))
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            return failure_of(error, live)
        return {
            "kind": "info",
            "token": info.token,
            "expires_on": info.expires_on,
            "token_type": info.token_type,
            "refresh_on": info.refresh_on,
            "token_requests": len(posts_of(live.client)),
            "record": record_of(credential),
        }


def lazy_result(step: Step) -> Json:
    with session(step) as live:
        try:
            provider = build_provider(step)
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            return failure_of(error, live)
        return {
            "kind": "callable",
            "callable": callable(provider),
            "token_requests": len(posts_of(live.client)),
        }


def token_response_result(step: Step) -> Json:
    observe = step.get("observe", "token")
    if observe == "lazy":
        return lazy_result(step)
    if observe == "info":
        return info_result(step)
    return attempt_result(step)


def exposes_identity(message: str, step: Step) -> bool:
    username = step.get("username", "")
    password = step.get("password", "")
    return (bool(username) and username in message) or (
        bool(password) and password in message
    )


def debug_of(live: Live, step: Step) -> Json:
    lines: list[Json] = []
    for message, args in live.debug:
        if not isinstance(message, str):
            continue
        if message.startswith("client_id="):
            filled = (*args, "", False, False)
            client_id = str(filled[0])
            username_set = bool(filled[1])
            password_set = bool(filled[2])
            lines.append(
                {
                    "kind": "credentials",
                    "client_id": client_id,
                    "username_set": username_set,
                    "password_set": password_set,
                    "exposes_identity": exposes_identity(message, step),
                }
            )
        elif message.startswith("token_provider "):
            lines.append(
                {
                    "kind": "provider",
                    "identity": "present",
                    "exposes_identity": exposes_identity(message, step),
                }
            )
    return lines


def warnings_of(caught: list[warnings.WarningMessage]) -> Json:
    return [
        {"category": warning.category.__name__, "message": str(warning.message)}
        for warning in caught
    ]


def construction_json(live: Live, step: Step) -> dict[str, Json]:
    return {
        "warnings": warnings_of(live.caught),
        "debug": debug_of(live, step),
        "token_requests": len(posts_of(live.client)),
    }


def construction_result(step: Step) -> Json:
    with session(step) as live:
        try:
            provider = build_provider(step)
            if int(step.get("calls", 0)) > 0:
                provider()
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            failure = failure_of(error, live)
            failure.update(construction_json(live, step))
            return failure
        return {"kind": "constructed", **construction_json(live, step)}


def string_value(value: Json | None, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def steps_of(value: Json | None) -> list[Step]:
    if not isinstance(value, list):
        raise ValueError("steps must be a list")
    steps: list[Step] = []
    for entry in value:
        if not isinstance(entry, dict):
            raise ValueError("each step must be an object")
        steps.append(cast(Step, entry))
    return steps


def run(input_value: dict[str, Json]) -> Json:
    action = string_value(input_value.get("action"), "action")
    steps = steps_of(input_value.get("steps"))
    if action in ("password_grant_form", "default_tenant"):
        return [grant_result(step) for step in steps]
    if action in ("invalid_grant", "password_scope_decoration"):
        return [attempt_result(step) for step in steps]
    if action == "token_response":
        return [token_response_result(step) for step in steps]
    if action in ("username_debug_warning", "username_constructor_failure"):
        return [construction_result(step) for step in steps]
    raise ValueError(f"unknown action {action}")
