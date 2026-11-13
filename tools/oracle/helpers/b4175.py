import json
import logging
import os
import random
import subprocess
from collections.abc import Callable
from contextlib import redirect_stdout
from io import StringIO
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
import uvicorn
from click.testing import CliRunner
from litellm._logging import verbose_logger, verbose_proxy_logger, verbose_router_logger
from litellm.proxy.common_utils.banner import show_banner
from litellm.proxy.common_utils.debug_utils import init_verbose_loggers
from litellm.proxy.proxy_cli import ProxyInitializationHelpers, run_server

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_BASELINE_LEVEL = logging.WARNING
_RANDOM_PORT = 31_000
_DEFAULT_PORT = 4_000


class HelperInput(TypedDict):
    operation: str
    host: NotRequired[str]
    port: NotRequired[int]
    body: NotRequired[str]
    model: NotRequired[str | None]
    api_base: NotRequired[str | None]
    args: NotRequired[list[str]]
    random_sample: NotRequired[int]
    worker_config: NotRequired[str]
    litellm_log: NotRequired[str]


class _CannedResponse:
    def __init__(self, body: Json) -> None:
        self._body = body

    def json(self) -> Json:
        return self._body


def _banner_text(_input_value: HelperInput) -> Json:
    stdout = StringIO()
    with redirect_stdout(stdout):
        show_banner()
    return stdout.getvalue()


def _health_prints_indented_json(input_value: HelperInput) -> Json:
    host = cast(str, input_value.get("host"))
    port = cast(int, input_value.get("port"))
    body = cast(str, input_value.get("body"))
    recorded: dict[str, Json] = {}

    def fake_get(*_args: object, **kwargs: object) -> _CannedResponse:
        recorded["url"] = cast(Json, kwargs.get("url"))
        return _CannedResponse(cast(Json, json.loads(body)))

    stdout = StringIO()
    with patch.object(httpx, "get", fake_get), redirect_stdout(stdout):
        ProxyInitializationHelpers._run_health_check(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            host, port
        )
    return {"url": recorded.get("url"), "stdout": stdout.getvalue()}


def _usage_message(stderr: str) -> str:
    lines = [line for line in stderr.splitlines() if line.strip()]
    if not lines:
        return ""
    return lines[-1].removeprefix("Error: ")


def _test_flag_invalid_value(input_value: HelperInput) -> Json:
    args = ["--test"]
    model = input_value.get("model")
    if model is not None:
        args.extend(["--model", model])
    result = CliRunner().invoke(run_server, args)
    error = "" if result.exception is None else str(result.exception)
    return {"stdout": result.stdout, "error": error}


def _validate_config_needs_path(_input_value: HelperInput) -> Json:
    result = CliRunner().invoke(run_server, ["--validate_config"])
    return {"exit_code": result.exit_code, "message": _usage_message(result.stderr)}


def _unknown_command(input_value: HelperInput) -> Json:
    result = CliRunner().invoke(run_server, cast(list[str], input_value.get("args")))
    return {"exit_code": result.exit_code, "message": _usage_message(result.stderr)}


def _ollama(input_value: HelperInput) -> Json:
    model = input_value.get("model")
    api_base = input_value.get("api_base")
    commands: list[Json] = []

    def fake_popen(command: list[str], *_args: object, **_kwargs: object) -> object:
        commands.append(cast(Json, list(command)))
        return object()

    stdout = StringIO()
    with patch.object(subprocess, "Popen", fake_popen), redirect_stdout(stdout):
        if model and "ollama" in model and api_base is None:
            ProxyInitializationHelpers._run_ollama_serve()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"commands": commands, "stdout": stdout.getvalue()}


def _port_fallback(input_value: HelperInput) -> Json:
    port = input_value.get("port", _DEFAULT_PORT)
    probed: list[Json] = []
    recorded: dict[str, Json] = {}

    def fake_in_use(candidate: int) -> bool:
        probed.append(candidate)
        return True

    def fake_randint(low: int, high: int) -> int:
        recorded["bounds"] = [low, high]
        return _RANDOM_PORT

    def fake_uvicorn_run(**kwargs: object) -> None:
        recorded["port"] = cast(Json, kwargs.get("port"))

    with (
        patch.dict(os.environ),
        patch.object(ProxyInitializationHelpers, "_is_port_in_use", fake_in_use),
        patch.object(random, "randint", fake_randint),
        patch.object(uvicorn, "run", fake_uvicorn_run),
    ):
        result = CliRunner().invoke(run_server, ["--port", str(port)])
    if "port" not in recorded:
        raise RuntimeError(f"run_server never reached uvicorn.run: {result.exception!r}")
    return {"port": recorded["port"], "probed": probed, "bounds": recorded.get("bounds")}


def _verbose(input_value: HelperInput) -> Json:
    worker_config = cast(str, input_value.get("worker_config"))
    loggers = {
        "package": verbose_logger,
        "router": verbose_router_logger,
        "proxy": verbose_proxy_logger,
    }
    for logger in loggers.values():
        logger.setLevel(_BASELINE_LEVEL)
    log_lines = StringIO()
    with patch.dict(os.environ, {"WORKER_CONFIG": worker_config}), redirect_stdout(log_lines):
        init_verbose_loggers()
    levels: dict[str, Json] = {
        name: None if logger.level == _BASELINE_LEVEL else logging.getLevelName(logger.level)
        for name, logger in loggers.items()
    }
    # DEBUG left on would print LiteLLM's shutdown log lines to stdout after the JSON answer
    for logger in loggers.values():
        logger.setLevel(_BASELINE_LEVEL)
    return levels


HANDLERS: dict[str, Callable[[HelperInput], Json]] = {
    "banner_text": _banner_text,
    "health_prints_indented_json": _health_prints_indented_json,
    "test_flag_invalid_value": _test_flag_invalid_value,
    "validate_config_needs_path": _validate_config_needs_path,
    "unknown_command": _unknown_command,
    "ollama_model_starts_serve": _ollama,
    "ollama_with_api_base_skips": _ollama,
    "port_4000_in_use_randomized": _port_fallback,
    "port_other_kept": _port_fallback,
    "verbose_debug_then_detailed": _verbose,
    "verbose_log_env_only_when_both_false": _verbose,
}


def run(input_value: dict[str, Json]) -> Json:
    request = cast(HelperInput, input_value)
    operation = request["operation"]
    if operation not in HANDLERS:
        raise ValueError(f"unknown b4175 operation: {operation}")
    return HANDLERS[operation](request)
