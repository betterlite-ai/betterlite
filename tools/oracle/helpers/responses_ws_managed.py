import importlib.util
import sys
from pathlib import Path
from typing import Protocol, TypedDict, runtime_checkable

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True


class Input(TypedDict):
    setup: dict[str, JsonValue]
    frames: list[str]
    disconnect: str
    scripts: list[JsonValue]
    quota: JsonValue
    sink: dict[str, JsonValue]


@runtime_checkable
class Adapter(Protocol):
    def run(self, input_value: Input) -> JsonValue: ...


def run(input_value: Input) -> JsonValue:
    module_path = Path(__file__).with_suffix("") / "support_pkg" / "__init__.py"
    spec = importlib.util.spec_from_file_location(
        "responses_ws_managed_support",
        module_path,
        submodule_search_locations=[str(module_path.parent)],
    )
    if spec is None or spec.loader is None:
        raise RuntimeError("managed WebSocket adapter module cannot be loaded")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    if not isinstance(module, Adapter):
        raise RuntimeError("managed WebSocket adapter has an invalid interface")
    return module.run(input_value)
