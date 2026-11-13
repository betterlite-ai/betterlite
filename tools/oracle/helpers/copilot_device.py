from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypedDict
from unittest.mock import patch

import httpx
from litellm.llms.github_copilot.authenticator import (
    DEFAULT_GITHUB_DEVICE_CODE_URL,
    Authenticator,
    GetAccessTokenError,
)
from pydantic import JsonValue


class Reply(TypedDict):
    status: int
    body: JsonValue


class Input(TypedDict):
    access_token_file: str | None
    device_replies: list[Reply]
    access_replies: list[Reply]
    fail_write: bool


class Fixture(Authenticator):
    def __init__(self, root: str, data: Input) -> None:
        self.requests: list[JsonValue] = []
        self.instructions: list[JsonValue] = []
        self.sleeps: list[JsonValue] = []
        self.device_replies: list[Reply] = list(data["device_replies"])
        self.access_replies: list[Reply] = list(data["access_replies"])
        self.path = Path(root) / "access-token"
        super().__init__()
        if data["access_token_file"] is not None:
            _ = self.path.write_text(data["access_token_file"])
        if data["fail_write"]:
            self.access_token_file = str(Path(root) / "missing" / "access-token")

    def take_reply(self, url: str) -> Reply:
        replies = (
            self.device_replies
            if url == DEFAULT_GITHUB_DEVICE_CODE_URL
            else self.access_replies
        )
        if not replies:
            raise httpx.ConnectError("scripted transport failure")
        return replies.pop(0)

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(int(seconds))

    def instruction(self, *_args: object, **_kwargs: object) -> None:
        self.instructions.append(str(_args[0]))


class Client:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    def post(
        self,
        url: str,
        headers: dict[str, str] | None = None,
        json: JsonValue = None,
    ) -> httpx.Response:
        self.fixture.requests.append(
            {"url": url, "headers": dict(headers or {}), "body": json}
        )
        reply = self.fixture.take_reply(url)
        request = httpx.Request(method="POST", url=url)
        return httpx.Response(
            status_code=reply["status"], json=reply["body"], request=request
        )


def content(path: Path) -> JsonValue:
    if not path.exists():
        return None
    return path.read_text()


def run(input_value: Input) -> JsonValue:
    with (
        TemporaryDirectory() as root,
        patch.dict("os.environ", {"GITHUB_COPILOT_TOKEN_DIR": root}),
    ):
        fixture = Fixture(root, input_value)
        client = Client(fixture)
        with (
            patch(
                "litellm.llms.github_copilot.authenticator._get_httpx_client",
                lambda: client,
            ),
            patch(
                "litellm.llms.github_copilot.authenticator.time.sleep", fixture.sleep
            ),
            patch("builtins.print", fixture.instruction),
        ):
            try:
                result: JsonValue = fixture.get_access_token()
                error_status: JsonValue = None
            except GetAccessTokenError as error:
                result = None
                error_status = error.status_code
        return {
            "result": result,
            "error_status": error_status,
            "file": content(fixture.path),
            "requests": fixture.requests,
            "instructions": fixture.instructions,
            "sleeps": fixture.sleeps,
        }
