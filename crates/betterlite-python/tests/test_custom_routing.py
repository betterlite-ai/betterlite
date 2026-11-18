import asyncio
import inspect
import subprocess
import sys
from collections.abc import Iterable
from typing import Final

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite._custom_routing import CustomRoutingStrategyBase

MODEL: Final = "gpt-4o"
RENAMED: Final = {"input": "input_value"}
KEYWORD_ONLY: Final = frozenset({"specific_deployment", "request_kwargs"})
RAW: Final = TypeAdapter(dict[str, list[tuple[str, str, str]]])
# strict mypy reads LiteLLM's bare list and dict annotations as Any; the probe runs in a subprocess
PROBE: Final = """
import inspect
import json
import sys

from betterlite._custom_routing import CustomRoutingStrategyBase as Ours
from litellm.types.router import CustomRoutingStrategyBase as Theirs


def rows(cls):
    method = getattr(cls, sys.argv[1])
    return [
        [p.name, p.kind.name, str(p)]
        for p in inspect.signature(method).parameters.values()
    ]


print(json.dumps({"ours": rows(Ours), "theirs": rows(Theirs)}))
"""


def probe(method: str) -> dict[str, list[tuple[str, str, str]]]:
    done = subprocess.run(
        [sys.executable, "-c", PROBE, method],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return RAW.validate_json(done.stdout)


def normalized(raw: Iterable[tuple[str, str, str]]) -> list[tuple[str, str, str]]:
    found: list[tuple[str, str, str]] = []
    for name, kind, text in raw:
        renamed = RENAMED.get(name, name)
        default = text.partition(" = " if " = " in text else "=")[2]
        found.append((renamed, "KEYWORD_ONLY" if renamed in KEYWORD_ONLY else kind, default))
    return found


def test_async_signature_matches() -> None:
    read = probe("async_get_available_deployment")
    names = [row[0] for row in read["ours"]]
    assert "input_value" in names
    assert "input" not in names
    assert normalized(read["ours"]) == normalized(read["theirs"])


def test_sync_signature_matches() -> None:
    read = probe("get_available_deployment")
    assert normalized(read["ours"]) == normalized(read["theirs"])


def test_keyword_only_arguments() -> None:
    for signature in (
        inspect.signature(CustomRoutingStrategyBase.async_get_available_deployment),
        inspect.signature(CustomRoutingStrategyBase.get_available_deployment),
    ):
        for name in KEYWORD_ONLY:
            assert signature.parameters[name].kind.name == "KEYWORD_ONLY"


class RecordingStrategy(CustomRoutingStrategyBase):
    @override
    def get_available_deployment(
        self, model: str, messages: list[dict[str, str]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> None: ...

    @override
    async def async_get_available_deployment(
        self, model: str, messages: list[dict[str, str]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> None: ...


def test_a_subclass_can_be_called() -> None:
    strategy = RecordingStrategy()
    assert strategy.get_available_deployment(MODEL) is None
    assert asyncio.run(strategy.async_get_available_deployment(MODEL)) is None
