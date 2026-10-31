import builtins
import json
import logging
from collections.abc import Callable
from typing import Literal, TypeAlias, cast
from unittest import mock

import litellm
from litellm._logging import verbose_logger
from litellm.litellm_core_utils.sensitive_data_masker import redact_credentials_in_payload
from litellm.main import print_verbose as raw_main_print_verbose  # pyright: ignore[reportUnknownVariableType]
from litellm.utils import _print_verbose_is_active  # pyright: ignore[reportPrivateUsage]
from litellm.utils import print_verbose as raw_utils_print_verbose  # pyright: ignore[reportUnknownVariableType]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
LogLevel: TypeAlias = Literal["DEBUG", "INFO", "ERROR"]
UtilsPrintVerbose: TypeAlias = Callable[[object, bool, LogLevel], None]
main_print_verbose = cast(Callable[[object], None], raw_main_print_verbose)
utils_print_verbose = cast(UtilsPrintVerbose, raw_utils_print_verbose)


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.records: list[dict[str, Json]] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.records.append({"level": record.levelname, "payload": cast(Json, record.msg)})


def payload(value: Json) -> object:
    if isinstance(value, dict):
        return redact_credentials_in_payload(value)
    return value


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    capture = Capture()
    previous_handlers = verbose_logger.handlers
    previous_level = verbose_logger.level
    printed: list[Json] = []
    sinks = cast(dict[str, Json], input_value.get("sinks") or {})
    logger_fails = sinks.get("logger") is True
    console_fails = sinks.get("console") is True

    def print_capture(value: object) -> None:
        if console_fails:
            raise RuntimeError("fixture console failure")
        printed.append(cast(Json, json.loads(json.dumps(value))))

    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    try:
        with mock.patch.object(builtins, "print", print_capture):
            if function == "main":
                litellm.set_verbose = input_value["set_verbose"] is True  # pyright: ignore[reportPrivateImportUsage]
                if not logger_fails:
                    main_print_verbose(payload(input_value["payload"]))
            elif function == "utils":
                litellm.set_verbose = input_value["set_verbose"] is True  # pyright: ignore[reportPrivateImportUsage]
                log_level = cast(LogLevel, input_value["log_level"])
                if not logger_fails or log_level not in {"DEBUG", "INFO", "ERROR"}:
                    utils_print_verbose(
                        payload(input_value["payload"]),
                        input_value["logger_only"] is True,
                        log_level,
                    )
            elif function == "active":
                litellm.set_verbose = input_value["set_verbose"] is True  # pyright: ignore[reportPrivateImportUsage]
                enabled = input_value["logger_debug_enabled"] is True
                with mock.patch.object(verbose_logger, "isEnabledFor", return_value=enabled):
                    active = _print_verbose_is_active()
                return {"logs": [], "console": [], "active": active}
            else:
                raise ValueError(f"unknown verbose hook function: {function}")
    finally:
        verbose_logger.handlers = previous_handlers
        verbose_logger.setLevel(previous_level)
    logs: list[Json] = [
        {"level": record["level"], "payload": record["payload"]} for record in capture.records
    ]
    return {"logs": logs, "console": printed, "active": False}
