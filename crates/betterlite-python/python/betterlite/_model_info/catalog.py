import json
from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict

import betterlite
from betterlite import _native


class _SyncRequest(TypedDict):
    entries: dict[str, JsonValue]
    removed: list[str]


def _canonical_texts(model_cost: dict[str, JsonValue]) -> dict[str, str]:
    return {
        key: json.dumps(value, sort_keys=True, separators=(",", ":"))
        for key, value in model_cost.items()
    }


class _CatalogState:
    def __init__(self) -> None:
        text = _native.bundled_model_cost_json()
        self.native: Final = _native.NativeCatalog(text)
        bundled: dict[str, JsonValue] = json.loads(text)
        bundled.pop("fallback_generalizations", None)
        self.generation = 0
        self.synced = _canonical_texts(bundled)

    def sync(self) -> None:
        current: dict[str, JsonValue] = betterlite.model_cost
        texts = _canonical_texts(current)
        entries: dict[str, JsonValue] = {
            key: current[key] for key, text in texts.items() if self.synced.get(key) != text
        }
        removed: list[str] = [key for key in self.synced if key not in texts]
        if not entries and not removed:
            return
        request: _SyncRequest = {"entries": entries, "removed": removed}
        self.native.set_runtime_entries(json.dumps(request))
        self.synced = texts
        self.generation += 1

    def adopt(self, model_cost: dict[str, JsonValue]) -> None:
        self.native.adopt(json.dumps(model_cost))
        self.synced = _canonical_texts(model_cost)
        self.generation += 1


catalog_state: Final = _CatalogState()


# ast-grep-ignore: no-long-names, no-one-line-functions
def get_model_cost_mutation_generation() -> int:
    return catalog_state.generation


def adopt_model_cost_map(model_cost: dict[str, JsonValue]) -> int:
    catalog_state.adopt(model_cost)
    betterlite.model_cost = model_cost
    return len(model_cost)


def git_blob_id(body: bytes) -> str:  # ast-grep-ignore: no-one-line-functions
    return _native.git_blob_id(body)
