import importlib
import importlib.abc
import importlib.machinery
import logging
import os
import sys
import time
import types
from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
from types import ModuleType, TracebackType
from typing import Literal, NotRequired, Self, TypedDict

import litellm_proxy_extras._logging as extras_logging
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class Arguments(TypedDict, total=False):
    positional: list[JsonValue]
    named: dict[str, JsonValue]


class ExceptionFixture(TypedDict):
    kind: str
    message: str


class FormatScenario(TypedDict):
    format: Literal["json", "text"]
    message: str
    level: str
    created: float
    offset_seconds: int
    arguments: NotRequired[Arguments]
    exception: NotRequired[ExceptionFixture]


class EnabledScenario(TypedDict):
    mode: Literal[
        "attribute",
        "missing_attribute",
        "import_error",
        "attribute_error",
        "other_error",
    ]
    json_logs: str | None
    global_enabled: bool


class Input(TypedDict):
    function: Literal["format", "json_enabled"]
    format_scenarios: list[FormatScenario]
    enabled_scenarios: list[EnabledScenario]


class FormattedRecord(TypedDict):
    message: str
    level: str
    timestamp: str
    stacktrace: NotRequired[str]


FORMATTED_RECORD: TypeAdapter[FormattedRecord] = TypeAdapter(FormattedRecord)


def format_scenarios(scenarios: list[FormatScenario]) -> JsonValue:
    results: list[JsonValue] = []
    for scenario in scenarios:
        apply_zone(scenario["offset_seconds"])
        if scenario["format"] == "json":
            results.append(json_result(scenario))
        else:
            results.append(text_result(scenario))
    return results


def json_result(scenario: FormatScenario) -> JsonValue:
    formatter = extras_logging.JsonFormatter()
    # the source formatter carries no annotations, so its known signature is declared here
    render: Callable[[logging.LogRecord], str] = formatter.format
    parsed = FORMATTED_RECORD.validate_json(render(log_record(scenario)))
    result: dict[str, JsonValue] = {
        "message": parsed["message"],
        "level": parsed["level"],
        "timestamp": parsed["timestamp"],
        "stacktrace": parsed.get("stacktrace"),
    }
    return result


def text_result(scenario: FormatScenario) -> JsonValue:
    formatter = logging.Formatter("%(asctime)s - %(name)s - %(levelname)s - %(message)s")
    return formatter.format(log_record(scenario))


def log_record(scenario: FormatScenario) -> logging.LogRecord:
    level = logging.getLevelNamesMapping()[scenario["level"].upper()]
    record = logging.LogRecord(
        "litellm_proxy_extras",
        level,
        "database_diagnostics.py",
        1,
        scenario["message"],
        arguments_for(scenario),
        exception_info(scenario),
    )
    record.created = scenario["created"]
    record.msecs = (record.created - int(record.created)) * 1000
    return record


def arguments_for(
    scenario: FormatScenario,
) -> tuple[JsonValue, ...] | Mapping[str, JsonValue] | None:
    arguments = scenario.get("arguments")
    if arguments is None:
        return None
    positional = arguments.get("positional")
    if positional is not None:
        return tuple(positional)
    named = arguments.get("named")
    if named is not None:
        return named
    return None


def exception_info(
    scenario: FormatScenario,
) -> tuple[type[BaseException], BaseException, None] | None:
    fixture = scenario.get("exception")
    if fixture is None:
        return None
    error = ValueError(fixture["message"])
    return (type(error), error, None)


def apply_zone(offset_seconds: int) -> None:
    absolute = abs(offset_seconds)
    hours, remainder = divmod(absolute, 3600)
    minutes, seconds = divmod(remainder, 60)
    if hours == 0 and minutes == 0 and seconds == 0:
        os.environ["TZ"] = "UTC0"
    else:
        sign = "-" if offset_seconds > 0 else "+"
        os.environ["TZ"] = f"BLT{sign}{hours:02d}:{minutes:02d}:{seconds:02d}"
    time.tzset()


def run(input_value: Input) -> JsonValue:
    with RestoredState():
        if input_value["function"] == "format":
            return format_scenarios(input_value["format_scenarios"])
        return [enabled_result(scenario) for scenario in input_value["enabled_scenarios"]]


def enabled_result(scenario: EnabledScenario) -> JsonValue:
    mode = scenario["mode"]
    if mode == "attribute":
        return attribute_enabled(scenario)
    if mode == "missing_attribute":
        return missing_attribute_enabled(scenario)
    if mode == "import_error":
        return failing_enabled(scenario, ImportError("litellm is unavailable"))
    if mode == "attribute_error":
        return failing_enabled(scenario, AttributeError("litellm"))
    return failing_enabled(scenario, TypeError("litellm"))


def attribute_enabled(scenario: EnabledScenario) -> bool:
    unload_litellm()
    set_optional("JSON_LOGS", "true" if scenario["global_enabled"] else None)
    importlib.import_module("litellm")
    return probe_flag(scenario.get("json_logs"))


def missing_attribute_enabled(scenario: EnabledScenario) -> bool:
    unload_litellm()
    sys.modules["litellm"] = types.ModuleType("litellm")
    return probe_flag(scenario.get("json_logs"))


def failing_enabled(scenario: EnabledScenario, error: BaseException) -> bool:
    unload_litellm()
    finder = FailingFinder(error)
    sys.meta_path.insert(0, finder)
    try:
        return probe_flag(scenario.get("json_logs"))
    finally:
        sys.meta_path.remove(finder)


def probe_flag(json_logs: str | None) -> bool:
    set_optional("JSON_LOGS", json_logs)
    os.environ["LITELLM_LOG"] = "INFO"
    extras_logging.logger.handlers.clear()
    importlib.reload(extras_logging)
    handler = extras_logging.logger.handlers[0]
    return handler.formatter.__class__.__name__ == "JsonFormatter"


class FailingFinder(importlib.abc.MetaPathFinder):
    def __init__(self, error: BaseException) -> None:
        self.error = error

    @override
    def find_spec(
        self,
        fullname: str,
        _path: Sequence[str] | None,
        _target: ModuleType | None = None,
        /,
    ) -> importlib.machinery.ModuleSpec | None:
        if fullname != "litellm":
            return None
        raise self.error


def unload_litellm() -> None:
    names = [
        name
        for name in sys.modules
        if name == "litellm" or name.startswith("litellm.")
    ]
    for name in names:
        sys.modules.pop(name, None)


def set_optional(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


class RestoredState(AbstractContextManager["RestoredState"]):
    def __init__(self) -> None:
        self.environment = os.environ.copy()
        self.litellm_modules = {
            name: module
            for name, module in sys.modules.items()
            if name == "litellm" or name.startswith("litellm.")
        }
        self.handlers = list(extras_logging.logger.handlers)
        self.level = extras_logging.logger.level
        self.meta_path = list(sys.meta_path)

    @override
    def __enter__(self) -> Self:
        return self

    @override
    def __exit__(
        self,
        error_type: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
        /,
    ) -> None:
        os.environ.clear()
        os.environ.update(self.environment)
        unload_litellm()
        sys.modules.update(self.litellm_modules)
        sys.meta_path[:] = self.meta_path
        extras_logging.logger.handlers.clear()
        extras_logging.logger.handlers.extend(self.handlers)
        extras_logging.logger.setLevel(self.level)
