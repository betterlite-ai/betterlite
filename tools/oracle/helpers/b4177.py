import os
import sys
from collections.abc import Callable
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from types import ModuleType, SimpleNamespace
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

from click import ClickException
from fastapi import FastAPI
from litellm.proxy import proxy_cli
from litellm.proxy.proxy_cli import ProxyInitializationHelpers

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class LaunchOptions(TypedDict):
    backend: str
    host: str
    port: int
    workers: int
    runtime_threads: NotRequired[int | None]
    certfile: NotRequired[str | None]
    keyfile: NotRequired[str | None]
    ciphers: NotRequired[str | None]
    max_requests: NotRequired[int | None]
    max_requests_jitter: NotRequired[int | None]
    metrics_port: NotRequired[int | None]


class HelperInput(TypedDict):
    function: str
    options: NotRequired[LaunchOptions]
    settings: NotRequired[dict[str, object] | None]
    workers: NotRequired[int]
    metrics_port: NotRequired[int | None]
    environment: NotRequired[dict[str, str]]


def _module(name: str) -> ModuleType:
    return ModuleType(name)


def _worker_environment(values: dict[str, str]) -> dict[str, str]:
    return values


def _capture_hypercorn(options: LaunchOptions) -> tuple[dict[str, Json], str]:
    captured: dict[str, Json] = {}
    config_module = _module("hypercorn.config")

    class Config:
        def __init__(self) -> None:
            self.bind: list[str] = []

    config_module.Config = Config  # type: ignore[reportAttributeAccessIssue]
    async_module = _module("hypercorn.asyncio")

    async def serve(_app: FastAPI, config: Config) -> None:
        captured["bind"] = cast(Json, config.bind)
        captured["certfile"] = getattr(config, "certfile", None)
        captured["keyfile"] = getattr(config, "keyfile", None)
        captured["ciphers"] = getattr(config, "ciphers", None)

    async_module.serve = serve  # type: ignore[reportAttributeAccessIssue]
    package = _module("hypercorn")
    package.__path__ = []
    stdout = StringIO()
    with (
        patch.dict(
            sys.modules,
            {
                "hypercorn": package,
                "hypercorn.config": config_module,
                "hypercorn.asyncio": async_module,
            },
        ),
        redirect_stdout(stdout),
    ):
        ProxyInitializationHelpers._init_hypercorn_server(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast(FastAPI, object()),
            options["host"],
            options["port"],
            cast(str, options.get("certfile")),
            cast(str, options.get("keyfile")),
            options.get("ciphers"),
        )
    return captured, stdout.getvalue()


def _capture_granian(options: LaunchOptions) -> tuple[dict[str, Json], str]:
    captured: dict[str, Json] = {}
    granian = _module("granian")

    class Granian:
        def __init__(self, **kwargs: object) -> None:
            for key, value in kwargs.items():
                captured[key] = str(value) if key in {"ssl_cert", "ssl_key"} else cast(Json, value)

        def serve(self) -> None:
            return None

    granian.Granian = Granian  # type: ignore[reportAttributeAccessIssue]
    constants = _module("granian.constants")
    constants.Interfaces = SimpleNamespace(ASGI="asgi")  # type: ignore[reportAttributeAccessIssue]
    granian.__path__ = []  # type: ignore[reportAttributeAccessIssue]
    stdout = StringIO()
    with (
        patch.dict(sys.modules, {"granian": granian, "granian.constants": constants}),
        redirect_stdout(stdout),
    ):
        ProxyInitializationHelpers._init_granian_server(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            options["host"],
            options["port"],
            options["workers"],
            options.get("certfile"),
            options.get("keyfile"),
            options.get("max_requests"),
            options.get("ciphers"),
            options.get("runtime_threads"),
        )
    return captured, stdout.getvalue()


def _capture_gunicorn(
    options: LaunchOptions, environment: dict[str, str]
) -> tuple[dict[str, Json], str]:
    captured: dict[str, Json] = {}
    base_module = _module("gunicorn.app.base")

    class BaseApplication:
        options: dict[str, object]
        cfg: SimpleNamespace
        application: object

        def __init__(self) -> None:
            self.cfg = SimpleNamespace(settings={})
            self.load_config()
            self.load()

        def load_config(self) -> None:
            return None

        def load(self) -> object:
            return self.application

        def run(self) -> None:
            for key, value in self.options.items():
                captured[key] = cast(Json, value)

    base_module.BaseApplication = BaseApplication  # type: ignore[reportAttributeAccessIssue]
    gunicorn = _module("gunicorn")
    app_module = _module("gunicorn.app")
    app_module.__path__ = []  # type: ignore[reportAttributeAccessIssue]
    gunicorn.__path__ = []  # type: ignore[reportAttributeAccessIssue]
    fork_guard = _module("litellm.rust_bridge.fork_guard")
    fork_guard.reserve_process_for_forking = lambda _owner: None  # type: ignore[reportAttributeAccessIssue]
    cleanup = _module("litellm.proxy.prometheus_cleanup")
    cleanup.mark_worker_exit = lambda _pid: None  # type: ignore[reportAttributeAccessIssue]
    modules = {
        "gunicorn": gunicorn,
        "gunicorn.app": app_module,
        "gunicorn.app.base": base_module,
        "litellm.rust_bridge.fork_guard": fork_guard,
        "litellm.proxy.prometheus_cleanup": cleanup,
    }
    stdout = StringIO()
    with (
        patch.dict(sys.modules, modules),
        patch.dict(os.environ, _worker_environment(environment), clear=True),
        patch.object(proxy_cli, "start_query_engine_reaper", lambda: None),
        redirect_stdout(stdout),
    ):
        ProxyInitializationHelpers._run_gunicorn_server(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            options["host"],
            options["port"],
            cast(FastAPI, object()),
            options["workers"],
            cast(str, options.get("certfile")),
            cast(str, options.get("keyfile")),
            options.get("max_requests"),
            options.get("max_requests_jitter"),
        )
    return captured, stdout.getvalue()


def _launch_plan(options: LaunchOptions, environment: dict[str, str]) -> Json:
    backend = options["backend"]
    try:
        if backend == "hypercorn":
            capture, _stdout = _capture_hypercorn(options)
        elif backend == "granian":
            capture, _stdout = _capture_granian(options)
        else:
            capture, _stdout = _capture_gunicorn(options, environment)
    except (ClickException, ModuleNotFoundError) as error:
        return {"error": str(error)}
    if backend == "hypercorn":
        tls_enabled = capture.get("certfile") is not None and capture.get("keyfile") is not None
        ciphers = capture.get("ciphers") if tls_enabled else None
        workers = options["workers"]
        preload = False
        cleanup = False
        timeout = None
        runtime_threads = None
        max_requests = None
        jitter = None
    elif backend == "granian":
        tls_enabled = capture.get("ssl_cert") is not None and capture.get("ssl_key") is not None
        ciphers = None
        workers = cast(int, capture["workers"])
        preload = False
        cleanup = False
        timeout = None
        runtime_threads = capture.get("runtime_threads")
        max_requests = None
        jitter = None
    else:
        tls_enabled = capture.get("certfile") is not None and capture.get("keyfile") is not None
        ciphers = None
        workers = cast(int, capture["workers"])
        preload = cast(bool, capture["preload"])
        cleanup = "child_exit" in capture
        timeout = cast(int, capture["timeout"])
        runtime_threads = None
        max_requests = capture.get("max_requests")
        jitter = capture.get("max_requests_jitter")
    warnings = _launch_warnings(options, backend)
    host = options["host"]
    port = options["port"]
    metrics_port = options.get("metrics_port")
    worker_suffix = "" if backend == "hypercorn" else f" with {workers} workers"
    metrics_suffix = f"; metrics port {metrics_port}" if metrics_port is not None else ""
    message = (
        f"Starting server on {host}:{port}{worker_suffix} using {backend.title()}{metrics_suffix}"
    )
    return cast(
        Json,
        {
            "backend": backend,
            "bind": f"{host}:{port}",
            "workers": workers,
            "timeout_seconds": timeout,
            "tls": {"enabled": tls_enabled, "ciphers": ciphers},
            "process": {"preload": preload, "child_exit_cleanup": cleanup},
            "runtime_threads": runtime_threads,
            "max_requests": max_requests,
            "max_requests_jitter": jitter,
            "start_message": message,
            "warnings": warnings,
        },
    )


def _launch_warnings(options: LaunchOptions, backend: str) -> list[str]:
    warnings: list[str] = []
    if backend == "granian" and options.get("max_requests") is not None:
        warnings.append("--max_requests_before_restart is not supported by Granian")
    if backend == "granian" and options.get("ciphers") is not None:
        warnings.append("--ciphers is not applied when using --run_granian")
    if (
        backend == "gunicorn"
        and options.get("max_requests_jitter") is not None
        and options.get("max_requests") is None
    ):
        warnings.append(
            "--max_requests_before_restart_jitter has no effect without "
            "--max_requests_before_restart"
        )
    return warnings


def _callback_configured(settings: dict[str, object] | None) -> bool:
    return ProxyInitializationHelpers._prometheus_callback_configured(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        settings
    )


def run(input_value: dict[str, Json]) -> Json:
    request = cast(HelperInput, input_value)
    function = request["function"]
    if function == "launch_plan":
        options = request.get("options")
        if options is None:
            raise ValueError("launch_plan needs options")
        return _launch_plan(options, request.get("environment", {}))
    if function == "callback_configured":
        return _callback_configured(request.get("settings"))
    if function == "prometheus_directory":
        settings = request.get("settings")
        workers = request.get("workers", 1)
        metrics_port = request.get("metrics_port")
        environment = request.get("environment", {})
        configured = environment.get("PROMETHEUS_MULTIPROC_DIR") or environment.get(
            "prometheus_multiproc_dir"
        )
        required = metrics_port is not None or (workers > 1 and _callback_configured(settings))
        if not required:
            return None
        cleanup = _module("litellm.proxy.prometheus_cleanup")

        def wipe_directory(_path: str) -> None:
            return None

        cleanup.wipe_directory = wipe_directory  # type: ignore[reportAttributeAccessIssue]

        def skip_makedirs(_name: str, _mode: int = 0o777, *, exist_ok: bool = False) -> None:
            _ = exist_ok

        with (
            patch.dict(sys.modules, {"litellm.proxy.prometheus_cleanup": cleanup}),
            patch.dict(os.environ, _worker_environment(environment), clear=True),
            patch("os.makedirs", skip_makedirs),
            redirect_stdout(StringIO()),
        ):
            setup_directory = cast(
                Callable[[int, dict[str, object] | None, int | None], str | None],
                ProxyInitializationHelpers._maybe_setup_prometheus_multiproc_dir,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            )
            path = setup_directory(workers, settings, metrics_port)
        if path is None:
            return None
        return {
            "path": str(Path(path)),
            "action": "Using existing" if configured else "Auto-created",
            "should_prepare": True,
        }
    raise ValueError("unknown B4177 function")
