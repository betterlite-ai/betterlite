from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
