import io
import json
import subprocess
from collections.abc import Mapping, Sequence
from contextlib import redirect_stdout
from functools import partial
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Final, TypedDict, cast
from unittest.mock import patch

import requests
from litellm.proxy.client.cli.commands import agents
from litellm.proxy.client.cli.commands.agents import AgentRunError
from pydantic import JsonValue

INSTALL_COMMAND: Final = "/usr/local/bin/betterlite statusline"
DEFAULT_INSTRUCTIONS: Final = "You are Codex."
EMPTY_CATALOG: Final = '{"models":[]}'
FAILED_CATALOG: Final = "error: invalid catalog"
_TIMEOUT: Final = 10
_DOCS: Final = (
    "https://docs.claude.com/en/docs/claude-code/setup",
    "https://developers.openai.com/codex/cli",
    "https://opencode.ai/docs",
    "https://pi.dev",
)


class Step(TypedDict, total=False):
    agent: str
    scenario: str
    command: list[str]
    binary: str | None
    models: list[dict[str, JsonValue]]
    stock: str


class _Response:
    def __init__(self, status: int, data: list[dict[str, JsonValue]]) -> None:
        self.status_code = status
        self.content = json.dumps({"data": data}).encode()

    def json(self) -> dict[str, JsonValue]:
        return cast(dict[str, JsonValue], json.loads(self.content))


class _Http:
    def __init__(self, models: Sequence[dict[str, JsonValue]]) -> None:
        self._models = list(models)

    def __call__(
        self, url: str, *, headers: Mapping[str, str], timeout: float
    ) -> requests.Response:
        if "Authorization" not in headers or timeout != _TIMEOUT:
            raise ValueError("unsupported proxy request")
        if not url.endswith("/v1/models"):
            return cast(requests.Response, _Response(404, []))
        return cast(requests.Response, _Response(200, self._models))


class _Debug:
    def __init__(self, answers: Sequence[dict[str, JsonValue]]) -> None:
        self._answers = list(answers)

    def __call__(  # noqa: PLR0913
        self,
        command: str | Sequence[str],
        env: Mapping[str, str],
        *,
        stdin: int,
        capture_output: bool,
        encoding: str,
        timeout: float,
    ) -> subprocess.CompletedProcess[str]:
        if not capture_output or encoding != "utf-8" or stdin != subprocess.DEVNULL:
            raise ValueError("unsupported debug invocation")
        if not env or timeout != _TIMEOUT or isinstance(command, str):
            raise ValueError("unsupported debug command")
        answer = self._answers.pop(0) if self._answers else {"status": 1}
        return subprocess.CompletedProcess(
            args=command,
            returncode=cast(int, answer.get("status", 1)),
            stdout=cast(str, answer.get("stdout", "")),
            stderr=cast(str, answer.get("stderr", "")),
        )


class _State:
    def __init__(self) -> None:
        self.warnings: list[str] = []
        self.launch: dict[str, JsonValue] | None = None

    def launch_record(self, path: str, args: Sequence[str], env: Mapping[str, str]) -> None:
        self.launch = {"binary": path, "argv": list(args), "env": dict(env)}


def _env_json(env: Mapping[str, str], root: str) -> dict[str, JsonValue]:
    result: dict[str, JsonValue] = {}
    for name, value in env.items():
        text = value.replace(root, "<tmp>")
        if name == "OPENCODE_CONFIG_CONTENT":
            try:
                result[name] = cast(JsonValue, json.loads(text))
            except json.JSONDecodeError:
                result[name] = text
        else:
            result[name] = text
    return result


def _launch_json(launch: dict[str, JsonValue] | None, root: str) -> JsonValue:
    if launch is None:
        return None
    arguments = cast(list[str], launch["argv"])
    argv = [argument.replace(root, "<tmp>") for argument in arguments]
    return cast(
        JsonValue,
        {
            "binary": cast(str, launch["binary"]).replace(root, "<tmp>"),
            "argv": argv,
            "env": _env_json(cast(dict[str, str], launch["env"]), root),
        },
    )


def _file_json(path: Path) -> JsonValue:
    if not path.exists():
        return None
    return cast(JsonValue, json.loads(path.read_text(encoding="utf-8")))


def _error_json(step: Step, failure: AgentRunError) -> dict[str, JsonValue]:
    if step.get("binary") is not None:
        return {"kind": "failed", "docs": None}
    message = str(failure)
    docs = next((url for url in _DOCS if url in message), None)
    return {"kind": "missing_binary", "docs": docs}


def _debug_answers(step: Step) -> list[dict[str, JsonValue]]:
    if step.get("agent") != "codex" or step.get("scenario") == "skip":
        return []
    first: dict[str, JsonValue] = {
        "status": 0, "stdout": step.get("stock", EMPTY_CATALOG), "stderr": ""
    }
    if step.get("scenario") == "read-back-failure":
        return [first, {"status": 2, "stdout": "", "stderr": FAILED_CATALOG}]
    return [first, {"status": 0, "stdout": EMPTY_CATALOG, "stderr": ""}]


def _run_step(step: Step) -> dict[str, JsonValue]:
    with TemporaryDirectory() as temporary:
        root = Path(temporary)
        root_text = str(root)
        codex_home = root / "codex"
        pi_dir = root / "pi"
        scenario = step.get("scenario")
        env = {"KEEP": "1"}
        if scenario == "existing-config":
            env["OPENCODE_CONFIG_CONTENT"] = '{"provider":{}}'
        env["HOME"] = str(root / "home")
        env["CODEX_HOME"] = str(codex_home)
        env["PI_CODING_AGENT_DIR"] = str(pi_dir)
        if scenario == "hooks":
            codex_home.mkdir(parents=True, exist_ok=True)
            _ = (codex_home / "config.toml").write_text("hooks.Stop = []\n", encoding="utf-8")
        instructions_path = root / "instructions.md"
        _ = instructions_path.write_text(
            DEFAULT_INSTRUCTIONS, encoding="utf-8"
        )
        http = _Http(step.get("models", []))
        debug = _Debug(_debug_answers(step))
        state = _State()
        warn = state.warnings.append

        def verify_key(base_url: str, api_key: str) -> None:
            agents.verify_proxy_key(base_url, api_key, get=http)

        def sync_models(
            binary: str,
            base_env: Mapping[str, str],
            base_url: str,
            api_key: str,
            skip_verify: bool,  # noqa: FBT001
        ) -> agents.ModelSyncResult:
            return agents.agent_model_sync_env(
                binary, base_env, base_url, api_key, skip_verify, get=http, run=debug
            )

        def prepare_pi_models(
            base_url: str, api_key: str, base_env: Mapping[str, str]
        ) -> tuple[str, ...]:
            return agents.prepare_pi(base_url, api_key, base_env, get=http)

        def prepare_codex_hooks(
            base_url: str, api_key: str, base_env: Mapping[str, str]
        ) -> tuple[str, ...]:
            return agents.prepare_codex(
                base_url, api_key, base_env, install=lambda: INSTALL_COMMAND, warn=warn
            )

        output = io.StringIO()
        error: dict[str, JsonValue] | None = None
        bound_sync = partial(agents.codex_model_sync_args, instructions_path=instructions_path)
        try:
            with redirect_stdout(output), patch.object(
                agents, "codex_model_sync_args", bound_sync
            ):
                agents.run_agent(
                    "http://gateway.test:4000/",
                    "sk-test",
                    step.get("command", []),
                    skip_verify=scenario == "skip",
                    base_env=env,
                    which=lambda _name: step.get("binary"),
                    verify=verify_key,
                    sync_models=sync_models,
                    warn=warn,
                    launcher=state.launch_record,
                    reattach_terminal=(lambda: None) if scenario != "missing-binary" else None,
                    preparers={"pi": prepare_pi_models, "codex": prepare_codex_hooks},
                )
        except AgentRunError as failure:
            error = _error_json(step, failure)
        notices = [line.replace(root_text, "<tmp>") for line in output.getvalue().splitlines()]
        warnings = [message.replace(root_text, "<tmp>") for message in state.warnings]
        return cast(
            dict[str, JsonValue],
            {
                "error": error,
                "warnings": warnings,
                "notices": notices,
                "launch": _launch_json(state.launch, root_text),
                "catalog": _file_json(codex_home / "litellm-models.json"),
                "pi_models": _file_json(pi_dir / "models.json"),
            },
        )


def run(input_value: dict[str, list[Step]]) -> JsonValue:
    return cast(JsonValue, {"steps": [_run_step(step) for step in input_value["steps"]]})
