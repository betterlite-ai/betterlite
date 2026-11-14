import logging
import os
from collections.abc import Iterator
from contextlib import contextmanager
from pydantic import JsonValue
from typing_extensions import override

from .models import ENVIRONMENT_KEYS


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


@contextmanager
def warnings_recorded() -> Iterator[list[JsonValue]]:
    recorder = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(recorder)
    try:
        yield recorder.messages
    finally:
        logger.removeHandler(recorder)


@contextmanager
def environment(values: dict[str, str] | None) -> Iterator[None]:
    given = {} if values is None else values
    saved = {name: os.environ.pop(name, None) for name in ENVIRONMENT_KEYS}
    os.environ.update({name: given[name] for name in ENVIRONMENT_KEYS if name in given})
    try:
        yield
    finally:
        for name in ENVIRONMENT_KEYS:
            os.environ.pop(name, None)
        os.environ.update({name: value for name, value in saved.items() if value is not None})
