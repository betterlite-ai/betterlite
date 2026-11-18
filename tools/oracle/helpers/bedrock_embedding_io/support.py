import os

from pydantic import JsonValue, TypeAdapter

BODY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


def set_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(environment)
