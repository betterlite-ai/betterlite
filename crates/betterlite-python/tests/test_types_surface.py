import json
import subprocess
import sys
from typing import Final

from pydantic import TypeAdapter

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
# re-exporting these 33 takes betterlite/types/llms/openai/__init__.py past the 300-line file limit
NOT_EXPORTED: Final[dict[str, frozenset[str]]] = {
    "llms.openai": frozenset(
        {
            "ALL_RESPONSES_API_TOOL_PARAMS",
            "AllEmbeddingInputValues",
            "AllMessageValues",
            "AllPromptValues",
            "AttachmentTool",
            "BatchJobStatus",
            "CREATE_FILE_REQUESTS_PURPOSE",
            "ChatCompletionAssistantContentValue",
            "ChatCompletionToolChoiceStringValues",
            "ChatCompletionToolChoiceValues",
            "EmbeddingInput",
            "FileContent",
            "FileTypes",
            "OpenAIAudioTranscriptionOptionalParams",
            "OpenAIChatCompletionFinishReason",
            "OpenAICreateFileRequestOptionalParams",
            "OpenAIFilesPurpose",
            "OpenAIImageEditOptionalParams",
            "OpenAIImageGenerationOptionalParams",
            "OpenAIImageVariationOptionalParams",
            "OpenAIMessageContent",
            "OpenAIMessageContentListBlock",
            "OpenAIRealtimeEvents",
            "OpenAIRealtimeStreamList",
            "PART_UNION_TYPES",
            "REASONING_EFFORT",
            "ResponsesAPIStatus",
            "ResponsesAPIStreamingResponse",
            "ResponsesTerminalEvent",
            "ToolMessageContentPart",
            "ValidAssistantMessageContentTypesLiteral",
            "ValidChatCompletionMessageContentTypesLiteral",
            "ValidUserMessageContentTypesLiteral",
        }
    ),
}
SURFACE_SCRIPT: Final = """
import ast
import importlib
import inspect
import json
import os
import pathlib
import sys
import typing

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"


def definition_names(tree):
    names = set()
    for node in tree.body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            names.add(node.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    names.add(target.id)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
    return names


def imported_names(tree):
    names = set()
    for node in tree.body:
        if not isinstance(node, ast.ImportFrom):
            continue
        if node.level == 0 and (node.module or "").split(".")[0] != "litellm":
            continue
        for alias in node.names:
            if alias.name != "*":
                names.add(alias.asname or alias.name)
    return names


def exported_names(tree):
    names = set()
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        declared = any(
            isinstance(target, ast.Name) and target.id == "__all__"
            for target in node.targets
        )
        if not declared or not isinstance(node.value, (ast.List, ast.Tuple)):
            continue
        for element in node.value.elts:
            if isinstance(element, ast.Constant) and isinstance(element.value, str):
                names.add(element.value)
    return names


def source_names(module):
    try:
        path = inspect.getsourcefile(module)
    except TypeError:
        return set()
    if path is None:
        return set()
    tree = ast.parse(pathlib.Path(path).read_text(encoding="utf-8"))
    declared = definition_names(tree) | imported_names(tree) | exported_names(tree)
    return {name for name in declared if not name.startswith("_")}


def runtime_names(module):
    names = set()
    for name, value in inspect.getmembers(module):
        if name.startswith("_") or inspect.ismodule(value) or value is typing.TYPE_CHECKING:
            continue
        origin = inspect.getmodule(value)
        if origin is None or origin.__name__.split(".")[0] == "litellm":
            names.add(name)
    return names


def public_names(module):
    return runtime_names(module) | source_names(module)


report = {}
for litellm_path, betterlite_path in json.loads(sys.argv[1]):
    theirs = importlib.import_module("litellm.types." + litellm_path)
    ours = importlib.import_module("betterlite.types." + betterlite_path)
    report[litellm_path] = sorted(public_names(theirs) - set(dir(ours)))
print(json.dumps(report))
"""


def test_names_match_litellm() -> None:
    completed = subprocess.run(
        [sys.executable, "-c", SURFACE_SCRIPT, json.dumps(MODULES)],
        check=True,
        capture_output=True,
        text=True,
    )
    report: dict[str, list[str]] = TypeAdapter(dict[str, list[str]]).validate_json(
        completed.stdout
    )
    missing: dict[str, list[str]] = {}
    for litellm_path, betterlite_path in MODULES:
        unexported = NOT_EXPORTED.get(litellm_path)
        exempt = NOT_PORTED if unexported is None else NOT_PORTED | unexported
        absent = sorted(set(report[litellm_path]) - exempt)
        if absent:
            missing[f"{litellm_path} as {betterlite_path}"] = absent
    assert not missing, missing
