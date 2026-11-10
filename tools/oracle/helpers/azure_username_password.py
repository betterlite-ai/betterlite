import os
import time
import warnings
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import NotRequired, Self, TypeAlias, TypedDict, cast
from unittest import mock

import azure.identity
from azure.core.credentials import TokenProvider
from azure.core.exceptions import ClientAuthenticationError
from azure.identity import UsernamePasswordCredential
from litellm._logging import verbose_logger
from litellm.llms.azure.common_utils import get_azure_ad_token_from_username_password

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
RECORD_PREFIX = "Failed to build AuthenticationRecord from unexpected identity token"
RESERVED_PREFIX = "Authentication failed: You cannot use any scope value that is reserved."
HTTP_ERROR_STATUS = 400
NOW = 1_700_000_000
CREDENTIAL_ARGUMENTS = {"client_id", "password", "username"}


class Reply(TypedDict):
    status: int
    body: str
    url_contains: NotRequired[str]
    error: NotRequired[str]


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


class RecordedRequest(TypedDict):
    method: str
    url: str
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

    def exchange(self, method: str, url: str, data: dict[str, object] | None) -> FixtureResponse:
        form = {name: str(value) for name, value in (data or {}).items()}
        self.requests.append({"method": method, "url": url, "form": form})
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

    def get(self, url: str, **_kwargs: object) -> FixtureResponse:
        return self.exchange("GET", url, None)

    def post(
        self, url: str, data: dict[str, object] | None = None, **_kwargs: object
    ) -> FixtureResponse:
        return self.exchange("POST", url, data)

    def posts(self) -> list[RecordedRequest]:
        return [request for request in self.requests if request["method"] == "POST"]


class Live:
    def __init__(
        self,
        client: FixtureClient,
        constructor: mock.MagicMock,
        clock: mock.MagicMock,
        caught: list[warnings.WarningMessage],
    ) -> None:
        self.client = client
        self.constructor = constructor
        self.clock = clock
        self.caught = caught


Action = Callable[[Callable[[], str], Live, Step], Json]

CREDENTIALS: list[UsernamePasswordCredential] = []
SCOPES: list[tuple[str, ...]] = []
DEBUG_CALLS: list[tuple[object, tuple[object, ...]]] = []
REAL_BEARER_TOKEN_PROVIDER = azure.identity.get_bearer_token_provider


def build_credential(client_id: str, username: str, password: str) -> UsernamePasswordCredential:
    credential = UsernamePasswordCredential(client_id, username, password)
    CREDENTIALS.append(credential)
    return credential


def recording_bearer_provider(credential: TokenProvider, *scopes: str) -> Callable[[], str]:
    SCOPES.append(scopes)
    return REAL_BEARER_TOKEN_PROVIDER(credential, *scopes)


def capture_debug(message: object, *args: object) -> None:
    DEBUG_CALLS.append((message, args))


@contextmanager
def session(step: Step) -> Generator[Live]:
    client = FixtureClient(list(step.get("replies", [])))
    CREDENTIALS.clear()
    SCOPES.clear()
    DEBUG_CALLS.clear()
    environment = dict(step.get("environment", {}))
    host = step.get("authority_host")
    if host is not None:
        environment["AZURE_AUTHORITY_HOST"] = host
    constructor = mock.MagicMock(side_effect=build_credential)
    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch("azure.identity._internal.msal_credentials.MsalClient", return_value=client),
        mock.patch.object(time, "time", return_value=float(NOW)) as clock,
        mock.patch.object(azure.identity, "UsernamePasswordCredential", constructor),
        mock.patch.object(azure.identity, "get_bearer_token_provider", recording_bearer_provider),
        mock.patch.object(verbose_logger, "debug", capture_debug),
        warnings.catch_warnings(record=True) as caught,
    ):
        warnings.simplefilter("always")
        yield Live(client, constructor, clock, caught)


def build_provider(step: Step) -> Callable[[], str]:
    client_id = step.get("client_id", "")
    username = step.get("username", "")
    password = step.get("password", "")
    scope = step.get("scope")
    if scope is None:
        return get_azure_ad_token_from_username_password(client_id, username, password)
    return get_azure_ad_token_from_username_password(client_id, username, password, scope)


def record_of(credential: UsernamePasswordCredential | None) -> Json:
    record = None if credential is None else credential._auth_record  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    if record is None:
        return None
    return {
        "authority": record.authority,
        "client_id": record.client_id,
        "home_account_id": record.home_account_id,
        "tenant_id": record.tenant_id,
        "username": record.username,
    }


def failure_detail(error: Exception, posts: int) -> tuple[str, str]:
    text = str(error)
    if isinstance(error, ClientAuthenticationError):
        if text.startswith(RECORD_PREFIX):
            return "auth_record", text
        if text.startswith(RESERVED_PREFIX):
            return "reserved_scope", text.split("\n", maxsplit=1)[0]
        return "authentication_failed", text if posts > 0 else ""
    if isinstance(error, KeyError):
        return (
            "missing_expires_in" if next(iter(error.args), None) == "expires_in" else "other"
        ), ""
    if isinstance(error, ValueError):
        return "constructor", text
    return "other", ""


def failure_of(error: Exception, live: Live) -> dict[str, Json]:
    posts = len(live.client.posts())
    reason, message = failure_detail(error, posts)
    if live.client.failed:
        reason, message = "transport", ""
    return {"kind": "error", "reason": reason, "message": message, "token_requests": posts}


def grant_action(provider: Callable[[], str], live: Live, _step: Step) -> Json:
    token = provider()
    posts = live.client.posts()
    if not posts:
        return {"kind": "error", "reason": "no_token_request", "message": "", "token_requests": 0}
    call = live.constructor.call_args
    discovery = [
        r["url"] for r in live.client.requests if "well-known/openid-configuration" in r["url"]
    ]
    form = posts[0]["form"]
    return cast(
        "Json",
        {
            "kind": "token",
            "credential_arguments": sorted(set(call.kwargs) & CREDENTIAL_ARGUMENTS),
            "scope_arguments": list(SCOPES[0]),
            "discovery": discovery[0] if discovery else None,
            "token_url": posts[0]["url"],
            "form": {name: value for name, value in form.items() if name != "scope"},
            "scope": sorted(form.get("scope", "").split()),
            "token": token,
        },
    )


def attempt_action(provider: Callable[[], str], live: Live, step: Step) -> Json:
    tokens: list[str] = []
    for index in range(int(step.get("calls", 1))):
        if index > 0:
            live.clock.return_value = float(NOW + int(step.get("advance", 0)))
        tokens.append(provider())
    return {
        "kind": "token",
        "tokens": cast("list[Json]", tokens),
        "token_requests": len(live.client.posts()),
        "record": record_of(CREDENTIALS[0] if CREDENTIALS else None),
    }


def info_action(_provider: Callable[[], str], live: Live, step: Step) -> Json:
    scope = step.get("scope")
    info = CREDENTIALS[0].get_token_info(DEFAULT_SCOPE if scope is None else scope)
    return {
        "kind": "info",
        "token": info.token,
        "expires_on": info.expires_on,
        "token_type": info.token_type,
        "refresh_on": info.refresh_on,
        "token_requests": len(live.client.posts()),
        "record": record_of(CREDENTIALS[0]),
    }


def lazy_action(provider: Callable[[], str], live: Live, _step: Step) -> Json:
    return {
        "kind": "callable",
        "callable": callable(provider),
        "token_requests": len(live.client.posts()),
    }


def construction_action(provider: Callable[[], str], _live: Live, step: Step) -> Json:
    if int(step.get("calls", 0)) > 0:
        provider()
    return {"kind": "constructed"}


def debug_of(step: Step) -> Json:
    lines: list[Json] = []
    for message, args in DEBUG_CALLS:
        if not isinstance(message, str):
            continue
        secrets = [step.get("username", ""), step.get("password", "")]
        exposes = any(secret != "" and secret in message for secret in secrets)
        if message.startswith("client_id="):
            filled = (*args, "", False, False)
            lines.append(
                {
                    "kind": "credentials",
                    "client_id": str(filled[0]),
                    "username_set": bool(filled[1]),
                    "password_set": bool(filled[2]),
                    "exposes_identity": exposes,
                }
            )
        elif message.startswith("token_provider "):
            lines.append({"kind": "provider", "identity": "present", "exposes_identity": exposes})
    return lines


def observe(step: Step, action: Action, *, construction: bool = False) -> Json:
    with session(step) as live:
        try:
            result = action(build_provider(step), live, step)
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            result = failure_of(error, live)
        if construction and isinstance(result, dict):
            result["token_requests"] = len(live.client.posts())
            result["debug"] = debug_of(step)
            result["warnings"] = cast(
                "Json",
                [{"category": w.category.__name__, "message": str(w.message)} for w in live.caught],
            )
        return result


ACTIONS: dict[str, Action] = {
    "password_grant_form": grant_action,
    "default_tenant": grant_action,
    "invalid_grant": attempt_action,
    "password_scope_decoration": attempt_action,
    "username_debug_warning": construction_action,
    "username_constructor_failure": construction_action,
}


def run(input_value: dict[str, Json]) -> Json:
    action = input_value.get("action")
    steps = input_value.get("steps")
    if not isinstance(action, str) or not isinstance(steps, list):
        raise ValueError("action must be a string and steps a list")
    token_response: dict[str, Action] = {"lazy": lazy_action, "info": info_action}
    results: list[Json] = []
    for entry in steps:
        step = cast(Step, entry)
        if action == "token_response":
            chosen = token_response.get(step.get("observe", "token"), attempt_action)
        else:
            chosen = ACTIONS[action]
        results.append(observe(step, chosen, construction=chosen is construction_action))
    return results
