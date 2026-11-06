import json
from datetime import UTC, datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypedDict, cast
from unittest.mock import patch

import httpx
from litellm.llms.github_copilot.authenticator import (
    Authenticator,
    GetAccessTokenError,
    GetAPIKeyError,
)
from pydantic import JsonValue

API_KEY_MARKER = "<api-key>"
ACCESS_MARKER = "<access-token>"


class Reply(TypedDict):
    status: int
    body: JsonValue


class Input(TypedDict):
    action: str
    now: int
    api_key_file: str | None
    access_token_file: str | None
    api_replies: list[Reply]
    fail_write: bool


class Clock:
    def __init__(self, seconds: int) -> None:
        self.seconds = seconds

    def now(self) -> datetime:
        return datetime.fromtimestamp(self.seconds, tz=UTC)


def marker(token: str) -> str:
    return "" if token == "" else API_KEY_MARKER


def normalize_authorization(value: str) -> str:
    scheme, separator, _token = value.partition(" ")
    return f"{scheme} {ACCESS_MARKER}" if separator else ACCESS_MARKER


def normalize_headers(headers: dict[str, str] | None) -> dict[str, str]:
    recorded = dict(headers or {})
    authorization = recorded.get("authorization")
    if authorization is not None:
        recorded["authorization"] = normalize_authorization(authorization)
    return recorded


def normalize_record(value: JsonValue) -> JsonValue:
    if not isinstance(value, dict):
        return value
    token = value.get("token")
    if isinstance(token, str):
        value["token"] = marker(token)
    return value


class Fixture(Authenticator):
    def __init__(self, root: str, data: Input) -> None:
        self.requests: list[JsonValue] = []
        self.replies: list[Reply] = list(data["api_replies"])
        self.key_path = Path(root) / "api-key.json"
        super().__init__()
        if data["access_token_file"] is not None:
            _ = (Path(root) / "access-token").write_text(data["access_token_file"])
        if data["api_key_file"] is not None:
            _ = self.key_path.write_text(data["api_key_file"])
        if data["fail_write"]:
            self.api_key_file = str(Path(root) / "persist" / "api-key.json")

    def take_reply(self) -> Reply | None:
        return self.replies.pop(0) if self.replies else None


class Client:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    def record(
        self,
        method: str,
        url: str,
        headers: dict[str, str] | None,
        body: JsonValue,
    ) -> None:
        self.fixture.requests.append(
            cast(
                JsonValue,
                {
                    "method": method,
                    "url": url,
                    "headers": normalize_headers(headers),
                    "body": body,
                },
            )
        )

    def get(self, url: str, headers: dict[str, str] | None = None) -> httpx.Response:
        self.record("GET", url, headers, None)
        reply = self.fixture.take_reply()
        if reply is None:
            raise httpx.ConnectError("scripted transport failure")
        return httpx.Response(
            status_code=reply["status"],
            json=reply["body"],
            request=httpx.Request(method="GET", url=url),
        )

    def post(
        self,
        url: str,
        headers: dict[str, str] | None = None,
        json: JsonValue = None,
    ) -> httpx.Response:
        self.record("POST", url, headers, json)
        raise httpx.ConnectError("scripted transport failure")


def run(input_value: Input) -> JsonValue:
    with (
        TemporaryDirectory() as root,
        patch.dict("os.environ", {"GITHUB_COPILOT_TOKEN_DIR": root}),
    ):
        fixture = Fixture(root, input_value)
        client = Client(fixture)
        result: JsonValue = None
        error_status: JsonValue = None
        with (
            patch(
                "litellm.llms.github_copilot.authenticator._get_httpx_client",
                lambda: client,
            ),
            patch(
                "litellm.llms.github_copilot.authenticator.datetime",
                Clock(input_value["now"]),
            ),
        ):
            try:
                if input_value["action"] == "get_api_key":
                    token = cast(str | None, fixture.get_api_key())
                    result = None if token is None else marker(token)
                elif input_value["action"] == "get_api_base":
                    result = fixture.get_api_base()
                else:
                    raise ValueError(f"unsupported Copilot key action: {input_value['action']}")
            except (GetAPIKeyError, GetAccessTokenError) as error:
                error_status = error.status_code
        content: JsonValue = None
        if fixture.key_path.exists():
            content = normalize_record(cast(JsonValue, json.loads(fixture.key_path.read_text())))
        return {
            "result": result,
            "error_status": error_status,
            "file": content,
            "requests": fixture.requests,
        }
