import subprocess
import sys
from typing import TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite.types.llms import oci

DESCRIBE_SCRIPT = """
import json
import os
from enum import Enum

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

import typing_extensions
from pydantic import BaseModel, TypeAdapter

import betterlite.types.llms.oci as betterlite_oci
import betterlite.types.llms.openai_evals as betterlite_evals
import betterlite.types.llms.watsonx as betterlite_watsonx
import litellm.types.llms.oci as litellm_oci
import litellm.types.llms.openai_evals as litellm_evals
import litellm.types.llms.watsonx as litellm_watsonx

ALIASES = {
    "oci": ("OCIRoles", "OCIContentPartUnion"),
    "openai_evals": ("DataSourceConfig", "GraderConfig", "RunDataSourceConfig"),
    "watsonx": (),
}


def collect_literals(node, found):
    if isinstance(node, dict):
        enum_values = node.get("enum")
        if isinstance(enum_values, list):
            found.update(json.dumps(item, sort_keys=True) for item in enum_values)
        if "const" in node:
            found.add(json.dumps(node["const"], sort_keys=True))
        for value in node.values():
            collect_literals(value, found)
    elif isinstance(node, list):
        for item in node:
            collect_literals(item, found)


def type_shape(node):
    if not isinstance(node, dict) or not node:
        return "json"
    if "$ref" in node:
        return "json" if node["$ref"].endswith("/JsonValue") else {"$ref": node["$ref"]}
    if "anyOf" in node:
        return {"anyOf": [type_shape(item) for item in node["anyOf"]]}
    kind = node.get("type")
    if kind == "object":
        extra = node.get("additionalProperties")
        if extra is None or extra is True:
            return "json"
        inner = type_shape(extra)
        return "json" if inner == "json" else {"object": inner}
    if kind == "array":
        return {"array": type_shape(node.get("items", {}))}
    return kind if kind is not None else "json"


def shape(schema):
    properties = schema.get("properties", {})
    literals = set()
    collect_literals(schema, literals)
    return {
        "keys": sorted(properties),
        "required": sorted(schema.get("required", [])),
        "defaults": {
            name: json.dumps(field["default"], sort_keys=True)
            for name, field in properties.items()
            if "default" in field
        },
        "literals": sorted(literals),
        "types": {name: type_shape(field) for name, field in properties.items()},
    }


def model_shape(model):
    described = shape(model.model_json_schema())
    described["bases"] = sorted(
        base.__name__
        for base in model.__bases__
        if base.__module__ == model.__module__ and not base.__name__.startswith("_")
    )
    return described


def describe(module, aliases):
    found = {"models": {}, "typeddicts": {}, "enums": {}, "aliases": {}}
    for name, value in vars(module).items():
        if name.startswith("_"):
            continue
        if isinstance(value, type) and value.__module__ == module.__name__:
            if issubclass(value, BaseModel):
                found["models"][name] = model_shape(value)
            elif issubclass(value, Enum):
                found["enums"][name] = {
                    "names": [member.name for member in value],
                    "schema": shape(TypeAdapter(value).json_schema()),
                }
            elif typing_extensions.is_typeddict(value):
                found["typeddicts"][name] = shape(TypeAdapter(value).json_schema())
    for name in aliases:
        found["aliases"][name] = (
            shape(TypeAdapter(vars(module)[name]).json_schema()) if name in vars(module) else None
        )
    return found


def sample_dumps(module):
    completion = module.OCICompletionPayload(
        compartmentId="ocid1.compartment.oc1..example",
        servingMode=module.OCIServingMode(
            servingType="ON_DEMAND", modelId="cohere.command-r-plus"
        ),
        chatRequest=module.OCIChatRequestPayload(
            apiFormat="COHERE",
            messages=[
                module.OCIMessage(
                    role="USER",
                    content=[
                        module.OCITextContentPart(text="what is in this image?"),
                        module.OCIImageContentPart(
                            imageUrl=module.OCIImageUrl(url="https://example.com/cat.png")
                        ),
                    ],
                    toolCalls=[
                        module.OCIToolCall(
                            id="call_1",
                            type="FUNCTION",
                            name="lookup",
                            arguments='{"query": "cat"}',
                        )
                    ],
                )
            ],
            isStream=False,
            maxTokens=64,
            temperature=0.2,
            topP=0.5,
            stop=["END"],
            seed=7,
            frequencyPenalty=0.1,
            presencePenalty=0.1,
            reasoningEffort="LOW",
            responseFormat={"type": "JSON_OBJECT"},
            toolChoice={"type": "FUNCTION", "name": "lookup"},
            logitBias={"50256": -100},
            logProbs=2,
        ),
    )
    cohere = module.CohereChatRequest(
        message="hello",
        chatHistory=[
            module.CohereUserMessage(message="hi"),
            module.CohereToolMessage(
                toolResults=[
                    module.CohereToolResult(
                        call=module.CohereToolCall(
                            name="lookup", parameters={"query": "cat"}
                        ),
                        outputs=[{"text": "a cat"}],
                    )
                ]
            ),
        ],
        maxTokens=32,
        temperature=0.2,
        topP=0.7,
        topK=5,
        frequencyPenalty=0.1,
        presencePenalty=0.1,
        stopSequences=["STOP"],
        seed=3,
        preambleOverride="be brief",
        documents=[{"id": "d1"}],
        searchQueriesOnly=False,
        isStream=False,
        citationQuality="ACCURATE",
    )
    return {
        "completion": json.dumps(completion.model_dump(exclude_none=True), sort_keys=True),
        "cohere": json.dumps(cohere.model_dump(exclude_none=True), sort_keys=True),
    }


print(json.dumps({
    "litellm": {
        "oci": describe(litellm_oci, ALIASES["oci"]),
        "openai_evals": describe(litellm_evals, ALIASES["openai_evals"]),
        "watsonx": describe(litellm_watsonx, ALIASES["watsonx"]),
    },
    "betterlite": {
        "oci": describe(betterlite_oci, ALIASES["oci"]),
        "openai_evals": describe(betterlite_evals, ALIASES["openai_evals"]),
        "watsonx": describe(betterlite_watsonx, ALIASES["watsonx"]),
    },
    "dumps": {
        "litellm": sample_dumps(litellm_oci),
        "betterlite": sample_dumps(betterlite_oci),
    },
}))
"""

OCI_MODELS = frozenset({
    "CohereChatBotMessage",
    "CohereChatDetails",
    "CohereChatRequest",
    "CohereChatResponse",
    "CohereChatResult",
    "CohereCitation",
    "CohereMessage",
    "CohereParameterDefinition",
    "CohereSearchQuery",
    "CohereStreamChunk",
    "CohereSystemMessage",
    "CohereTool",
    "CohereToolCall",
    "CohereToolMessage",
    "CohereToolResult",
    "CohereUsage",
    "CohereUserMessage",
    "OCIChatRequestPayload",
    "OCIChatResponse",
    "OCICompletionPayload",
    "OCICompletionResponse",
    "OCICompletionTokenDetails",
    "OCIContentPart",
    "OCIEmbedRequest",
    "OCIEmbedResponse",
    "OCIEmbedUsage",
    "OCIImageContentPart",
    "OCIImageUrl",
    "OCIMessage",
    "OCIPromptTokensDetails",
    "OCIResponseChoice",
    "OCIResponseUsage",
    "OCIServingMode",
    "OCIStreamChunk",
    "OCIStreamDelta",
    "OCITextContentPart",
    "OCIToolCall",
    "OCIToolDefinition",
})
OCI_ENUMS = frozenset({"OCIVendors"})
OCI_ALIASES = frozenset({"OCIRoles", "OCIContentPartUnion"})

EVALS_MODELS = frozenset({
    "CancelEvalResponse",
    "CancelRunResponse",
    "DeleteEvalResponse",
    "Eval",
    "ListEvalsResponse",
    "ListRunsResponse",
    "PerTestingCriteriaResult",
    "ResultCounts",
    "Run",
    "RunDeleteResponse",
})
EVALS_TYPEDDICTS = frozenset({
    "CompletionConfig",
    "CreateEvalRequest",
    "CreateRunRequest",
    "CustomGraderConfig",
    "DataSourceConfigCustom",
    "DataSourceConfigLogs",
    "DataSourceConfigStoredCompletions",
    "DataSourceDatasetConfig",
    "DataSourceInlineConfig",
    "DataSourceSampleSetConfig",
    "GroundTruthGraderConfig",
    "LLMAsJudgeGraderConfig",
    "ListEvalsParams",
    "ListRunsParams",
    "UpdateEvalRequest",
})
EVALS_ALIASES = frozenset({"DataSourceConfig", "GraderConfig", "RunDataSourceConfig"})

WATSONX_TYPEDDICTS = frozenset({
    "WatsonXAPIParams",
    "WatsonXAudioTranscriptionRequestBody",
    "WatsonXCredentials",
})
WATSONX_ENUMS = frozenset({"WatsonXAIEndpoint", "WatsonXModelPattern"})


class Shape(TypedDict):
    keys: list[str]
    required: list[str]
    defaults: dict[str, str]
    literals: list[str]
    types: dict[str, JsonValue]


class ModelShape(Shape):
    bases: list[str]


class EnumShape(TypedDict):
    names: list[str]
    schema: Shape


class ModuleNames(TypedDict):
    models: dict[str, ModelShape]
    typeddicts: dict[str, Shape]
    enums: dict[str, EnumShape]
    aliases: dict[str, Shape | None]


class NamesReport(TypedDict):
    litellm: dict[str, ModuleNames]
    betterlite: dict[str, ModuleNames]
    dumps: dict[str, dict[str, str]]


@pytest.fixture(scope="module")
def report() -> NamesReport:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
    )
    parsed: NamesReport = TypeAdapter(NamesReport).validate_json(completed.stdout)
    return parsed


def test_oci_names(report: NamesReport) -> None:
    expected = report["litellm"]["oci"]
    assert set(expected["models"]) == OCI_MODELS
    assert set(expected["enums"]) == OCI_ENUMS
    assert set(expected["aliases"]) == OCI_ALIASES
    assert not expected["typeddicts"]
    assert report["betterlite"]["oci"] == expected


def test_evals_names(report: NamesReport) -> None:
    expected = report["litellm"]["openai_evals"]
    assert set(expected["models"]) == EVALS_MODELS
    assert set(expected["typeddicts"]) == EVALS_TYPEDDICTS
    assert set(expected["aliases"]) == EVALS_ALIASES
    assert not expected["enums"]
    assert report["betterlite"]["openai_evals"] == expected


def test_watsonx_names(report: NamesReport) -> None:
    expected = report["litellm"]["watsonx"]
    assert set(expected["typeddicts"]) == WATSONX_TYPEDDICTS
    assert set(expected["enums"]) == WATSONX_ENUMS
    assert not expected["models"]
    assert not expected["aliases"]
    assert report["betterlite"]["watsonx"] == expected


def test_oci_wire_dumps(report: NamesReport) -> None:
    dumps = report["dumps"]
    assert dumps["betterlite"]["completion"] == dumps["litellm"]["completion"]
    assert dumps["betterlite"]["cohere"] == dumps["litellm"]["cohere"]


def test_role_names_are_enforced() -> None:
    tool_payload: dict[str, JsonValue] = {"toolResults": []}
    invalid_tool_payload: dict[str, JsonValue] = {**tool_payload, "role": "USER"}
    assert oci.CohereUserMessage(role="USER").role == "USER"
    assert oci.CohereToolMessage.model_validate(tool_payload).role == "TOOL"
    with pytest.raises(ValidationError, match="role") as user:
        oci.CohereUserMessage(role="SYSTEM")
    assert user.value.error_count() == 1
    with pytest.raises(ValidationError, match="role") as chatbot:
        oci.CohereChatBotMessage(role="USER")
    assert chatbot.value.error_count() == 1
    with pytest.raises(ValidationError, match="role") as system:
        oci.CohereSystemMessage(role="USER")
    assert system.value.error_count() == 1
    with pytest.raises(ValidationError, match="role") as tool:
        oci.CohereToolMessage.model_validate(invalid_tool_payload)
    assert tool.value.error_count() == 1
