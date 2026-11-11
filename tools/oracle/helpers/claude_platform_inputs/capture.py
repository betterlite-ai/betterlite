import logging
from collections.abc import Generator
from contextlib import contextmanager

from litellm._logging import verbose_logger
from typing_extensions import override

from .types import DROP_MARKER, IGNORE_MARKER, Json


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.entries: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.entries.append(warning_entry(record.getMessage()))


@contextmanager
def captured_warnings() -> Generator[list[Json], None, None]:
    handler = Warnings()
    verbose_logger.addHandler(handler)
    try:
        yield handler.entries
    finally:
        verbose_logger.removeHandler(handler)


def warning_entry(message: str) -> Json:
    if DROP_MARKER in message:
        _, _, listed = message.partition(DROP_MARKER)
        tuple_text, _, _ = listed.partition(" from the request body")
        inner = tuple_text.strip()[1:-1].rstrip(",")
        names: list[Json] = [] if not inner else [name.strip("'\"") for name in inner.split(", ")]
        return {"action": "drop", "params": names}
    _, _, type_text = message.partition(IGNORE_MARKER)
    return {"action": "ignore", "type_name": type_text.split(";", 1)[0]}
