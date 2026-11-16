from typing import Final, TypeAlias

import litellm
from pydantic import JsonValue, TypeAdapter

# every adapter module imports litellm, as the oracle's helper rule asks
_ = litellm.__name__

Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
