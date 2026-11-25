import importlib.util
import sys
from pathlib import Path
from types import ModuleType
from typing import TypedDict

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True


Input = TypedDict("Input", {"setup": dict[str, JsonValue], "frames": list[str],
    "disconnect": str, "scripts": list[JsonValue], "quota": JsonValue,
    "sink": dict[str, JsonValue]})


class AdapterModule(ModuleType):
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
    module = AdapterModule(spec.name)
    module.__spec__ = spec
    module.__package__ = spec.parent
    module.__dict__["__path__"] = [str(module_path.parent)]
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module.run(input_value)
