import logging
import os
import sys
from collections.abc import Generator, Sequence
from contextlib import contextmanager
from importlib.abc import Loader, MetaPathFinder
from importlib.machinery import ModuleSpec
from importlib.util import find_spec
from types import ModuleType
from typing import TypeAlias, cast, override
from unittest.mock import patch

from litellm._logging import verbose_logger
from litellm.integrations.langfuse.langfuse import (
    LangFuseLogger,
    parse_langfuse_debug,
    raise_if_unusable_prompt_cache_ttl,
    resolve_langfuse_credentials,  # pyright: ignore[reportUnknownVariableType]
    resolve_langfuse_host,
    warn_if_upstream_langfuse_configured,
)
from litellm.integrations.langfuse.langfuse_mock_client import should_use_langfuse_mock
from litellm.litellm_core_utils.initialize_dynamic_callback_params import (
    validate_langfuse_environment_value,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
STUBBED_PREFIXES = ("langfuse", "opentelemetry.exporter", "opentelemetry.sdk")
class Stub:
    def __init__(self, *args: object, **kwargs: object) -> None:
        del args, kwargs


class StubMeta(type):
    def __getattr__(cls, name: str) -> object:
        if name.startswith("__"):
            raise AttributeError(name)
        return stub_class(name)


def stub_class(name: str) -> type[Stub]:
    return StubMeta(name, (Stub,), {})


class StubModule(ModuleType):
    def __getattr__(self, name: str) -> object:
        if name.startswith("__"):
            raise AttributeError(name)
        return stub_class(name)


# The venv keeps opentelemetry-sdk and the langfuse package out, and langfuse_sdk.py
# imports both at the top even though these readers only touch os.environ.
class SdkStubs(MetaPathFinder, Loader):
    @override
    def find_spec(
        self,
        fullname: str,
        path: Sequence[str] | None,
        target: ModuleType | None = None,
    ) -> ModuleSpec | None:
        if fullname.startswith(STUBBED_PREFIXES):
            return ModuleSpec(fullname, self, is_package=True)
        return None

    @override
    def create_module(self, spec: ModuleSpec) -> ModuleType:
        return StubModule(spec.name)

    @override
    def exec_module(self, module: ModuleType) -> None:
        return None


if any(find_spec(prefix) is None for prefix in STUBBED_PREFIXES):
    sys.meta_path.append(SdkStubs())


class Recorder(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.records: list[tuple[int, str]] = []

    def filter(self, record: logging.LogRecord) -> bool:
        if record.levelno in (logging.INFO, logging.WARNING):
            self.records.append((record.levelno, record.getMessage()))
        return True

    def take(self) -> tuple[list[str], list[str], list[str]]:
        records, self.records = self.records, []
        warnings: list[str] = []
        details: list[str] = []
        infos: list[str] = []
        for level, message in records:
            if level == logging.WARNING:
                fixed, _, detail = message.partition(": ")
                warnings.append(fixed)
                details.append(detail)
            else:
                infos.append(message)
        return warnings, details, infos


@contextmanager
def recording() -> Generator[Recorder]:
    recorder = Recorder()
    previous_level = verbose_logger.level
    previous_handlers = verbose_logger.handlers
    previous_propagate = verbose_logger.propagate
    # INFO records go to stdout through LiteLLM's handler, which would corrupt the JSON answer
    verbose_logger.setLevel(logging.INFO)
    verbose_logger.handlers = []
    verbose_logger.propagate = False
    verbose_logger.filters.insert(0, recorder)
    try:
        yield recorder
    finally:
        verbose_logger.removeFilter(recorder)
        verbose_logger.handlers = previous_handlers
        verbose_logger.propagate = previous_propagate
        verbose_logger.setLevel(previous_level)


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast("dict[str, Json]", value)


def string_map(value: Json, name: str) -> dict[str, str]:
    entries = object_value(value, name)
    result: dict[str, str] = {}
    for key, item in entries.items():
        if not isinstance(item, str):
            raise ValueError(f"{name}.{key} must be a string")
        result[key] = item
    return result


def optional_text(value: Json, name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string or null")
    return value


def observed(recorder: Recorder, value: dict[str, Json]) -> Json:
    warnings, details, infos = recorder.take()
    value["warnings"] = cast("Json", warnings)
    value["warning_details"] = cast("Json", details)
    value["infos"] = cast("Json", infos)
    return value


def credentials_value(step: dict[str, Json]) -> Json:
    allow = step["allow_env_credentials"]
    if not isinstance(allow, bool):
        raise ValueError("allow_env_credentials must be a bool")
    public_key, secret_key, host = cast(
        "tuple[str | None, str | None, str]",
        resolve_langfuse_credentials(
            langfuse_public_key=optional_text(step.get("public_key"), "public_key"),
            langfuse_secret=optional_text(step.get("secret"), "secret"),
            langfuse_secret_key=optional_text(step.get("secret_key"), "secret_key"),
            langfuse_host=optional_text(step.get("host"), "host"),
            allow_env_credentials=allow,
        ),
    )
    return {"public_key": public_key, "secret_key": secret_key, "host": host}


def environment_value(step: dict[str, Json]) -> dict[str, Json]:
    explicit = optional_text(step.get("explicit"), "explicit")
    override = explicit.strip() if explicit is not None else None
    try:
        if override:
            validate_langfuse_environment_value(override)
            return {"ok": True, "value": override}
        resolved = LangFuseLogger.resolve_deployment_environment()
    except ValueError:
        return {"ok": False}
    return {"ok": True, "value": resolved}


def debug_and_mock() -> Json:
    result: dict[str, Json] = {
        "debug": parse_langfuse_debug(os.environ.get("LANGFUSE_DEBUG")),
        "mock": should_use_langfuse_mock(),
    }
    warn_if_upstream_langfuse_configured()
    return result


def sdk_reading(reading: str) -> Json:
    from litellm.integrations.langfuse.langfuse_sdk import (  # noqa: PLC0415 - the langfuse package is stubbed above
        configured_flush_at,
        configured_max_retries,
        configured_prompt_cache_ttl,
        configured_release,
        configured_sample_rate,
        configured_timeout,
    )

    if reading == "prompt_cache_ttl":
        raise_if_unusable_prompt_cache_ttl()
    match reading:
        case "flush_at":
            result: Json = configured_flush_at()
        case "sample_rate":
            result = configured_sample_rate()
        case "max_retries":
            result = configured_max_retries()
        case "timeout":
            result = configured_timeout()
        case "prompt_cache_ttl":
            result = configured_prompt_cache_ttl()
        case "release":
            result = configured_release()
        case "debug_and_mock":
            result = debug_and_mock()
        case _:
            raise ValueError(f"unknown reading: {reading}")
    return result


def run_step(function: str, reading: str | None, step: dict[str, Json], recorder: Recorder) -> Json:
    environment = string_map(step.get("environment", {}), "environment")
    with patch.dict(os.environ, environment, clear=True):
        match function:
            case "credentials":
                return observed(recorder, {"ok": True, "value": credentials_value(step)})
            case "host":
                host = resolve_langfuse_host(optional_text(step.get("host"), "host"))
                return observed(recorder, {"ok": True, "value": host})
            case "environment":
                return observed(recorder, environment_value(step))
            case "flush_interval":
                default = step["default"]
                if not isinstance(default, int) or isinstance(default, bool):
                    raise ValueError("default must be an int")
                seconds = LangFuseLogger._get_langfuse_flush_interval(default)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                return observed(recorder, {"ok": True, "value": seconds})
            case "read":
                if reading is None:
                    raise ValueError("a read call names its reading")
                try:
                    value = sdk_reading(reading)
                except ValueError:
                    return observed(recorder, {"ok": False})
                return observed(recorder, {"ok": True, "value": value})
            case _:
                raise ValueError(f"unknown function: {function}")


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    steps = input_value["steps"]
    if not isinstance(function, str) or not isinstance(steps, list):
        raise ValueError("function must be a string and steps a list")
    reading = optional_text(input_value.get("reading"), "reading")
    with recording() as recorder:
        return [
            run_step(function, reading, object_value(step_value, "step"), recorder)
            for step_value in steps
        ]
