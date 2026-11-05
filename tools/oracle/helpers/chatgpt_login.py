import json
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypedDict, cast
from unittest.mock import patch

import httpx
from litellm.llms.chatgpt.authenticator import Authenticator, JsonObject
from litellm.llms.chatgpt.common_utils import ChatGPTAuthError
from pydantic import JsonValue


class Reply(TypedDict):
    status: int | None
    body: str


class Input(TypedDict):
    record: str | None
    replacement: str | None
    now: float
    replies: list[Reply]
    repeat: Reply | None


class Client:
    def __init__(self, data: Input) -> None:
        self.replies: list[Reply] = list(data["replies"])
        self.repeat: Reply | None = data["repeat"]
        self.requests: list[JsonValue] = []

    def post(
        self,
        url: str,
        json: dict[str, JsonValue] | None = None,
        headers: dict[str, str] | None = None,
        content: str | None = None,
    ) -> httpx.Response:
        content_type = headers["Content-Type"] if headers else "application/json"
        body: JsonValue = json if json is not None else content
        self.requests.append({"url": url, "content_type": content_type, "body": body})
        reply = self.replies.pop(0) if self.replies else self.repeat
        if reply is None or reply["status"] is None:
            raise httpx.ConnectError("unreachable")
        return httpx.Response(
            reply["status"],
            content=reply["body"].encode(),
            request=httpx.Request("POST", url),
        )


class Fixture(Authenticator):
    def __init__(self, root: str, data: Input) -> None:
        self.path: Path = Path(root) / "auth.json"
        self.data: Input = data
        self.now: float = data["now"]
        self.sleeps: list[JsonValue] = []
        self.writes: list[JsonValue] = []
        self.events: list[JsonValue] = []
        self.display: list[JsonValue] = []
        super().__init__()
        if data["record"] is not None:
            _ = self.path.write_text(data["record"])

    def _write_auth_file(self, data: JsonObject) -> None:
        self.writes.append(dict(data))
        super()._write_auth_file(data)

    def sleep(self, seconds: float) -> None:
        self.now += seconds
        self.sleeps.append(seconds)
        replacement = self.data["replacement"]
        if replacement is not None:
            _ = self.path.write_text(replacement)
            self.data["replacement"] = None

    def warning(self, *_args: object) -> None:
        self.events.append("warning")

    def error(self, *_args: object) -> None:
        self.events.append("error")

    def show(self, *args: object, **_kwargs: object) -> None:
        self.display.append(str(args[0]))


def content(path: Path) -> JsonValue:
    if not path.exists():
        return None
    text = path.read_text()
    try:
        return cast(JsonValue, json.loads(text))
    except json.JSONDecodeError:
        return text


def run(input_value: Input) -> JsonValue:
    with (
        TemporaryDirectory() as root,
        patch.dict("os.environ", {"CHATGPT_TOKEN_DIR": root, "CHATGPT_AUTH_FILE": "auth.json"}),
    ):
        fixture = Fixture(root, input_value.copy())
        client = Client(input_value)
        token: str | None = None
        error: JsonValue = None
        with (
            patch("litellm.llms.chatgpt.authenticator.time.time", lambda: fixture.now),
            patch("litellm.llms.chatgpt.authenticator.time.sleep", fixture.sleep),
            patch("litellm.llms.chatgpt.authenticator._get_httpx_client", lambda: client),
            patch("builtins.print", fixture.show),
            patch("litellm.llms.chatgpt.authenticator.verbose_logger.warning", fixture.warning),
            patch("litellm.llms.chatgpt.authenticator.verbose_logger.error", fixture.error),
        ):
            try:
                token = fixture.get_access_token()
            except ChatGPTAuthError as exc:
                error = {"class": type(exc).__name__, "status": exc.status_code}
        return {
            "token": token,
            "error": error,
            "requests": client.requests,
            "sleeps": fixture.sleeps,
            "file": content(fixture.path),
            "writes": fixture.writes,
            "events": fixture.events,
            "display": fixture.display,
        }
