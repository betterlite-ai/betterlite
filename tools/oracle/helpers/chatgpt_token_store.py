import json
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypedDict, cast
from unittest.mock import patch

from litellm.llms.chatgpt.authenticator import Authenticator, JsonObject
from pydantic import JsonValue


class Input(TypedDict):
    record: str | None
    replacement: str | None
    now: float
    fail_write: bool
    actions: list[str | dict[str, float]]


class Fixture(Authenticator):
    def __init__(self, root: str, data: Input) -> None:
        self.path: Path = Path(root) / "auth.json"
        self.data: Input = data
        self.now: float = data["now"]
        self.sleeps: list[JsonValue] = []
        self.writes: list[JsonValue] = []
        self.events: list[JsonValue] = []
        self.reads: int = 0
        super().__init__()
        if data.get("record") is not None:
            _ = self.path.write_text(data["record"] or "")

    def _read_auth_file(self) -> JsonObject | None:
        self.reads += 1
        return super()._read_auth_file()

    def _write_auth_file(self, data: JsonObject) -> None:
        self.writes.append(dict(data))
        self.auth_file = (
            str(self.path / "missing") if self.data.get("fail_write") else str(self.path)
        )
        super()._write_auth_file(data)
        self.auth_file = str(self.path)

    def sleep(self, seconds: float) -> None:
        self.now += seconds
        self.sleeps.append(seconds)
        replacement = self.data.get("replacement")
        if replacement is not None:
            _ = self.path.write_text(replacement)
            self.data["replacement"] = None

    def warning(self, *_args: object) -> None:
        self.events.append("warning")

    def error(self, *_args: object) -> None:
        self.events.append("error")

    def build(self, record: JsonObject | None) -> JsonValue:
        tokens = {key: value for key, value in (record or {}).items() if isinstance(value, str)}
        return dict(self._build_auth_record(tokens))

    def action(self, action: str | dict[str, float]) -> JsonValue:
        if isinstance(action, dict):
            return self._wait_for_access_token(action["wait"])
        if action == "account":
            return self.get_account_id()
        if action == "record":
            self._record_device_code_request()
            return None
        record = self._read_auth_file()
        if action == "read":
            result = dict(record) if record is not None else None
        elif action == "build":
            result = self.build(record)
        else:
            result = self.evaluate(action, record)
        return result

    def evaluate(self, action: str, record: JsonObject | None) -> JsonValue:
        if action == "cooldown":
            return self._get_device_code_cooldown_remaining(record)
        token = record.get("access_token") if record else None
        return self._is_token_expired(record, token) if record and isinstance(token, str) else True


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
        with (
            patch("litellm.llms.chatgpt.authenticator.time.time", lambda: fixture.now),
            patch("litellm.llms.chatgpt.authenticator.time.sleep", fixture.sleep),
            patch(
                "litellm.llms.chatgpt.authenticator.verbose_logger.warning",
                fixture.warning,
            ),
            patch(
                "litellm.llms.chatgpt.authenticator.verbose_logger.error",
                fixture.error,
            ),
        ):
            results = [fixture.action(action) for action in input_value["actions"]]
        return {
            "results": results,
            "file": content(fixture.path),
            "writes": fixture.writes,
            "reads": fixture.reads,
            "events": fixture.events,
            "sleeps": fixture.sleeps,
        }
