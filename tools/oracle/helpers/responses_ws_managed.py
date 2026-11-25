from importlib import import_module
from pathlib import Path
from typing import Final, Protocol, TypedDict, cast

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True


Input = TypedDict("Input", {"setup": dict[str, JsonValue], "frames": list[str],
    "disconnect": str, "scripts": list[JsonValue], "quota": JsonValue,
    "sink": dict[str, JsonValue]})


class Support(Protocol):
    def run(self, input_value: Input) -> JsonValue: ...


__path__ = [str(Path(__file__).with_suffix(""))]

SUPPORT: Final[Support] = cast(Support, import_module("responses_ws_managed.support_pkg"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L2862-L2890

run: Final = SUPPORT.run
