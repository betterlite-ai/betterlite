import importlib
import json
import os
import signal
import sys
from collections.abc import Callable, Mapping
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from tempfile import TemporaryDirectory
from types import FrameType
from typing import Final, Literal, NotRequired, TypedDict

import click
import requests
from click.testing import CliRunner
from litellm.litellm_core_utils.cli_keyring import (
    KeyringDisabled,
    SecretErase,
    SecretRead,
    SecretVault,
    SecretWrite,
)
from litellm.proxy.client.cli.commands import auth as auth_module
from litellm.proxy.client.cli.commands import claude_settings as claude_settings_module
from litellm.proxy.client.cli.commands import up as up_module
from litellm.proxy.client.cli.commands.agents import AgentRunError, verify_proxy_key
from litellm.proxy.client.cli.commands.auth import CliContextObj
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

Handler = Callable[[int, FrameType | None], None] | int | None
BASE_URL: Final = "http://gateway.test:4000"
STOP_NUMBERS: Final[dict[str | None, int]] = {
    "sigint": signal.SIGINT,
    "sigterm": signal.SIGTERM,
    "atexit": signal.SIGTERM,
}
JSON: Final = TypeAdapter(JsonValue)


class TokenRecord(TypedDict, total=False):
    base_url: str
    key: str
    timestamp: float
    expires_at: float
    refresh_token: str


class Input(TypedDict):
    operation: Literal["fresh_login", "read_backup", "down", "up"]
    stored: NotRequired[TokenRecord]
    login_token: NotRequired[TokenRecord]
    api_key: NotRequired[str]
    api_key_from_token_file: NotRequired[bool]
    tty: NotRequired[bool]
    settings: NotRequired[str]
    backup: NotRequired[str]
    verify_status: NotRequired[int]
    stop: NotRequired[Literal["sigint", "sigterm", "atexit"]]


class Vault(SecretVault):
    @override
    def read(self) -> SecretRead:
        return KeyringDisabled()

    @override
    def write(self, blob: str) -> SecretWrite:
        del blob
        return KeyringDisabled()

    @override
    def erase(self) -> SecretErase:
        return KeyringDisabled()


class TerminalInput(StringIO):
    @override
    def isatty(self) -> bool:
        return True


def fire(handler: Handler, signalnum: int) -> None:
    if callable(handler):
        handler(signalnum, None)


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def json_file(path: Path) -> JsonValue:
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8")
    try:
        return JSON.validate_json(text)
    except ValidationError:
        return text


def token_value(script: "Script") -> JsonValue:
    found = auth_module.load_token(vault=script.vault)
    if found is None:
        return None
    return JSON.validate_json(json.dumps(found))


def failure(exception: BaseException, name: str, root: str) -> dict[str, JsonValue]:
    message = str(exception).replace(root, "<tmp>")
    return {"class": name, "message": message}


class ScriptedGet:
    def __init__(self, status: int) -> None:
        self.status = status
        self.probe: dict[str, JsonValue] | None = None

    def __call__(self, url: str, headers: Mapping[str, str], timeout: float) -> requests.Response:
        self.probe = {
            "url": url,
            "timeout": timeout,
            "authorization": headers.get("Authorization"),
        }
        response = requests.Response()
        response.status_code = self.status
        return response


class Script:
    def __init__(self, root: Path, input_value: Input) -> None:
        self.root = root
        self.input_value = input_value
        self.settings = root / ".claude" / "settings.json"
        self.backup = root / ".litellm" / "claude_settings_backup.json"
        self.vault = Vault()
        self.stop = input_value.get("stop")
        status = input_value.get("verify_status")
        self.verify_status = 200 if status is None else status
        self.snapshot: dict[str, JsonValue] | None = None
        self.handler: Handler = None
        self.probe: dict[str, JsonValue] | None = None
        self.logins = 0
        self.login_command = click.Command("login", callback=self.login)

    def login(self, *, config_claude: bool, pkce: bool) -> None:
        del config_claude, pkce
        self.logins += 1
        token = self.input_value.get("login_token")
        if token is not None:
            write_text(self.root / ".litellm" / "token.json", json.dumps(token))

    def verify(self, base_url: str, api_key: str, *, get: JsonValue | None = None) -> None:
        del get
        scripted = ScriptedGet(self.verify_status)
        verify_proxy_key(base_url, api_key, get=scripted)
        self.probe = scripted.probe

    def signal(self, signalnum: int, handler: Handler) -> Handler:
        self.handler = handler
        self.snapshot = {
            "settings": json_file(self.settings),
            "backup": json_file(self.backup),
        }
        if signalnum == STOP_NUMBERS.get(self.stop):
            fire(handler, signalnum)
        return handler


def write_initial_files(root: Path, input_value: Input) -> None:
    settings_text = input_value.get("settings")
    if settings_text is not None:
        write_text(root / ".claude" / "settings.json", settings_text)
    backup_text = input_value.get("backup")
    if backup_text is not None:
        write_text(root / ".litellm" / "claude_settings_backup.json", backup_text)
    stored = input_value.get("stored")
    if stored is not None:
        write_text(root / ".litellm" / "token.json", json.dumps(stored))


def prepare(root: Path, input_value: Input) -> Script:
    os.environ["HOME"] = str(root)
    _ = importlib.reload(claude_settings_module)
    _ = importlib.reload(up_module)
    write_initial_files(root, input_value)
    script = Script(root, input_value)
    up_module.verify_proxy_key = script.verify  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/client/cli/commands/agents.py#L653
    up_module.login = script.login_command  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/client/cli/commands/up.py#L140
    signal.signal = script.signal
    return script


def context_value(script: Script) -> CliContextObj:
    input_value = script.input_value
    return {
        "base_url": BASE_URL,
        "api_key": input_value.get("api_key"),
        "api_key_from_token_file": input_value.get("api_key_from_token_file") is True,
        "secret_vault": script.vault,
    }


def answer(script: Script, stdout: str, error: JsonValue) -> JsonValue:
    root = str(script.root)
    settings = script.root / ".claude" / "settings.json"
    backup = script.root / ".litellm" / "claude_settings_backup.json"
    result: dict[str, JsonValue] = {
        "stdout": stdout.replace(root, "<tmp>"),
        "error": error,
        "login_calls": script.logins,
        "probe": script.probe,
        "stopped_by": script.stop,
        "settings": json_file(settings),
        "backup": json_file(backup),
        "token": token_value(script),
        "snapshot": script.snapshot,
    }
    return result


def run_fresh_login(input_value: Input) -> JsonValue:
    with TemporaryDirectory() as temporary:
        root = Path(temporary)
        script = prepare(root, input_value)
        if input_value.get("tty") is True:
            sys.stdin = TerminalInput()
        context = click.Context(up_module.up, obj=context_value(script))
        stdout = StringIO()
        error: JsonValue = None
        try:
            with redirect_stdout(stdout):
                up_module.ensure_fresh_login(context)
        except AgentRunError as caught:
            error = failure(caught, "AgentRunError", str(root))
        except up_module.UpError as caught:
            error = failure(caught, "UpError", str(root))
        except claude_settings_module.ClaudeSettingsError as caught:
            error = failure(caught, "ClaudeSettingsError", str(root))
        return answer(script, stdout.getvalue(), error)


def run_read_backup(input_value: Input) -> JsonValue:
    with TemporaryDirectory() as temporary:
        root = Path(temporary)
        script = prepare(root, input_value)
        error: JsonValue = None
        try:
            up_module.read_backup(backup_path=script.backup)
        except up_module.UpError as caught:
            error = failure(caught, "UpError", str(root))
        return answer(script, "", error)


def run_command(input_value: Input) -> JsonValue:
    with TemporaryDirectory() as temporary:
        root = Path(temporary)
        script = prepare(root, input_value)
        if input_value["operation"] == "down":
            result = CliRunner().invoke(up_module.down, standalone_mode=False)
        else:
            result = CliRunner().invoke(
                up_module.up, obj=context_value(script), standalone_mode=False
            )
        error: JsonValue = None
        if result.exception is not None:
            error = failure(result.exception, "ClickException", str(root))
        return answer(script, result.stdout, error)


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    if operation == "fresh_login":
        return run_fresh_login(input_value)
    if operation == "read_backup":
        return run_read_backup(input_value)
    return run_command(input_value)
