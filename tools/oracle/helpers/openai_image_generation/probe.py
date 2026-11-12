from dataclasses import dataclass, field
from typing import Final, Literal

import litellm
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

NOW: Final = 1_700_000_000
SUCCESS_MIN: Final = 200
SUCCESS_MAX: Final = 300


@dataclass
class Answer:
    status: int = 200
    headers: dict[str, str] = field(default_factory=dict)
    body: str = ""


@dataclass
class Probe:
    model: str = ""
    prompt: str = ""
    optional_params: dict[str, Json] = field(default_factory=dict)
    non_default_params: dict[str, Json] = field(default_factory=dict)
    existing_params: dict[str, Json] = field(default_factory=dict)
    drop_params: bool = False
    reply: Answer = field(default_factory=Answer)
    failure: Literal["timeout", "failed"] | None = None
    api_key: str | None = None
    api_base: str | None = None
    organization: str | None = None
    headers: dict[str, str] | None = None
    timeout: int | None = None
    is_async: bool = False


PROBE: Final = TypeAdapter(Probe)
DOCUMENT: Final = TypeAdapter(dict[str, Json])
