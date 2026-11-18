import importlib
import inspect
import typing
from types import ModuleType
from typing import Final

import pytest
from pydantic import JsonValue

MODULES: Final[list[tuple[str, str]]] = [
    ("llms.openai", "llms.openai"),
    ("llms.bedrock", "llms.bedrock"),
    ("llms.anthropic", "llms.anthropic"),
    ("llms.gemini", "llms.gemini"),
    ("llms.vertex_ai", "llms.vertex_ai"),
    ("llms.vertex_ai_gemini_transcription", "llms.vertex_ai_audio"),
    ("llms.vertex_ai_speech_to_text", "llms.vertex_ai_audio"),
    ("llms.vertex_ai_text_to_speech", "llms.vertex_ai_audio"),
    ("llms.openai_evals", "llms.openai_evals"),
    ("guardrails", "guardrails"),
    ("integrations.prometheus", "integrations.prometheus"),
    ("interactions", "interactions"),
    ("mcp", "mcp"),
    ("proxy.policy_engine", "proxy.policy_engine"),
    ("proxy.guardrails.guardrail_hooks", "proxy.guardrails.guardrail_hooks"),
]
NOT_PORTED: Final = frozenset({"HttpxBinaryResponseContent"})


def public_names(module: ModuleType) -> set[str]:
    members: list[tuple[str, JsonValue]] = inspect.getmembers(module)
    names: set[str] = set()
    for name, member in members:
        if name.startswith("_"):
            continue
        if inspect.ismodule(member):
            continue
        if member is typing.TYPE_CHECKING:
            continue
        origin = inspect.getmodule(member)
        if origin is None or origin.__name__.split(".")[0] == "litellm":
            names.add(name)
    return names


@pytest.mark.parametrize(("litellm_path", "betterlite_path"), MODULES)
def test_names_match_litellm(litellm_path: str, betterlite_path: str) -> None:
    theirs = importlib.import_module(f"litellm.types.{litellm_path}")
    ours = importlib.import_module(f"betterlite.types.{betterlite_path}")
    missing = public_names(theirs) - NOT_PORTED - set(dir(ours))
    assert not missing, f"{betterlite_path} is missing {sorted(missing)}"
