import logging
from collections.abc import Callable
from typing import TypeAlias, cast, override

from litellm.litellm_core_utils import fallback_generalizations as engine
from litellm.litellm_core_utils.fallback_generalizations import match_routing_generalization

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# LiteLLM annotates these with a bare list and dict, so strict mode needs the JSON element type
set_rules = cast(Callable[[list[Json] | None], None], engine.set_fallback_generalizations)
installed_rules = cast(Callable[[], Json], engine.get_fallback_generalization_rules)
match_capabilities = cast(Callable[[str], Json], engine.match_capability_generalizations)
match_fill_missing = cast(Callable[[str, str], Json], engine.match_fill_missing_generalizations)


class WarningCapture(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.warnings: list[Json] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        subject = record.args[0] if isinstance(record.args, tuple) and record.args else None
        self.warnings.append({"subject": str(subject), "message": record.getMessage()})
        return True


def strings(value: Json, name: str) -> list[str]:
    items = [item for item in value if isinstance(item, str)] if isinstance(value, list) else []
    if not isinstance(value, list) or len(items) != len(value):
        raise ValueError(f"{name} must be a list of strings")
    return items


def probe_pair(probe: Json) -> list[str]:
    if not isinstance(probe, dict) or set(probe) != {"model", "provider"}:
        raise ValueError("each fill_missing probe needs exactly model and provider")
    return strings([probe["model"], probe["provider"]], "fill_missing")


def run(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"rules", "routing", "capabilities", "fill_missing"}:
        raise ValueError("input fields must be rules, routing, capabilities, and fill_missing")
    rules = input_value["rules"]
    probes = input_value["fill_missing"]
    if rules is not None and not isinstance(rules, list):
        raise ValueError("rules must be a list or null")
    if not isinstance(probes, list):
        raise ValueError("fill_missing must be a list")
    pairs = [probe_pair(probe) for probe in probes]
    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    # first in line, since LiteLLM's own filter rewrites the record and drops its arguments
    logger.filters.insert(0, capture)
    set_rules(rules)
    logger.removeFilter(capture)
    routing = strings(input_value["routing"], "routing")
    capabilities = strings(input_value["capabilities"], "capabilities")
    return {
        "rules": installed_rules(),
        "warnings": capture.warnings,
        "routing": [match_routing_generalization(model) for model in routing],
        "capabilities": [match_capabilities(model) for model in capabilities],
        "fill_missing": [match_fill_missing(model, provider) for model, provider in pairs],
    }
