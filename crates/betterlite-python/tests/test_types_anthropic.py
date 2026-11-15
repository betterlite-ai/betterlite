import json
import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite.types.llms.anthropic import (
    ANTHROPIC_API_HEADERS,
    ANTHROPIC_API_ONLY_HEADERS,
)
from betterlite.types.llms.anthropic.anthropic_tool_search import get_tool_search_beta_header
from betterlite.types.llms.anthropic.headers import (
    AnthropicBetaHeaderValues,
    AnthropicHostedToolNames,
)

NAMES: Final[dict[str, frozenset[str]]] = {
    "anthropic": frozenset({
        "ANTHROPIC_ADVISOR_TOOL_TYPE", "ANTHROPIC_API_HEADERS", "ANTHROPIC_API_ONLY_HEADERS",
        "ANTHROPIC_BETA_HEADER_VALUES", "ANTHROPIC_EFFORT_BETA_HEADER",
        "ANTHROPIC_FINE_GRAINED_TOOL_STREAMING_BETA_HEADER", "ANTHROPIC_HOSTED_TOOLS",
        "ANTHROPIC_MID_CONVERSATION_OUTPUT_CONFIG_BETA_HEADER",
        "ANTHROPIC_MID_CONVERSATION_TOOL_CHANGES_BETA_HEADER", "ANTHROPIC_OAUTH_BETA_HEADER",
        "ANTHROPIC_OAUTH_TOKEN_PREFIX", "ANTHROPIC_PROMPT_CACHING_SCOPE_BETA_HEADER",
        "ANTHROPIC_THINKING_DISPLAY_UPDATES_BETA_HEADER", "ANTHROPIC_TOOL_SEARCH_BETA_HEADER",
        "ANTHROPIC_TOOL_SEARCH_TOOL_TYPES", "AllAnthropicMessageValues",
        "AllAnthropicPassThroughMessageValues", "AllAnthropicToolsValues",
        "AnthopicMessagesAssistantMessageParam", "AnthropicAdvisorTool",
        "AnthropicChatCompletionUsageBlock", "AnthropicCitation",
        "AnthropicCitationCharLocation", "AnthropicCitationPageLocation",
        "AnthropicCodeExecutionTool", "AnthropicCompaction", "AnthropicComputerTool",
        "AnthropicContainer", "AnthropicContentParamSource",
        "AnthropicContentParamSourceContent", "AnthropicContentParamSourceFileId",
        "AnthropicContentParamSourceText", "AnthropicContentParamSourceUrl",
        "AnthropicFinishReason", "AnthropicHostedTools", "AnthropicInputSchema",
        "AnthropicMcpServerTool", "AnthropicMcpServerToolConfiguration",
        "AnthropicMemoryTool", "AnthropicMessagesAssistantMessageValues",
        "AnthropicMessagesContainerUploadParam", "AnthropicMessagesDocumentParam",
        "AnthropicMessagesImageParam", "AnthropicMessagesRequest",
        "AnthropicMessagesRequestOptionalParams", "AnthropicMessagesSystemMessageParam",
        "AnthropicMessagesTextParam", "AnthropicMessagesTool", "AnthropicMessagesToolChoice",
        "AnthropicMessagesToolResultContent", "AnthropicMessagesToolResultParam",
        "AnthropicMessagesToolUseParam", "AnthropicMessagesUserMessageParam",
        "AnthropicMessagesUserMessageValues", "AnthropicMetadata", "AnthropicOutputConfig",
        "AnthropicOutputSchema", "AnthropicOutputTokensDetails", "AnthropicResponse",
        "AnthropicResponseContentBlockRedactedThinking", "AnthropicResponseContentBlockText",
        "AnthropicResponseContentBlockThinking", "AnthropicResponseContentBlockToolUse",
        "AnthropicResponseUsageBlock", "AnthropicStopDetails", "AnthropicSystemMessageContent",
        "AnthropicThinkingParam", "AnthropicToolSearchToolBM25", "AnthropicToolSearchToolRegex",
        "AnthropicWebSearchTool", "AnthropicWebSearchUserLocation", "AppliedEdit",
        "ChatCompletionCachedContent", "ChatCompletionRedactedThinkingBlock",
        "ChatCompletionThinkingBlock", "ChatCompletionUsageBlock", "CitationsObject",
        "CodeExecutionToolCaller", "CompactionBlock", "ContentBlockContentBlockDict",
        "ContentBlockDelta", "ContentBlockStart", "ContentBlockStartText",
        "ContentBlockStartToolUse", "ContentBlockStop", "ContentCitationsBlockDelta",
        "ContentJsonBlockDelta", "ContentTextBlockDelta", "ContentThinkingBlockDelta",
        "ContentThinkingSignatureBlockDelta", "ContextManagementResponse", "DirectToolCaller",
        "MessageBlockDelta", "MessageChunk", "MessageDelta", "MessageStartBlock",
        "PromptCacheBreakpoint", "ServerToolUsage", "StreamingContentBlockDeltaType",
        "TextBlock", "ToolCaller", "ToolReference", "ToolUseBlock", "UsageDelta",
        "UsageIteration",
    }),
    "anthropic_request": frozenset({"AnthropicMetadata"}),
    "anthropic_response": frozenset({
        "AnthropicMessagesResponse", "AnthropicResponseContentBlock",
        "AnthropicResponseContentBlockText", "AnthropicResponseContentBlockToolUse",
        "AnthropicResponseRedactedThinkingBlock", "AnthropicResponseTextBlock",
        "AnthropicResponseThinkingBlock", "AnthropicResponseToolUseBlock",
        "AnthropicStopDetails", "AnthropicUsage", "CompactionBlock",
        "ContextManagementResponse", "ServerToolUsage", "UsageIteration",
    }),
    "anthropic_skills": frozenset({
        "CreateSkillRequest", "CreateSkillVersionRequest", "DeleteSkillResponse",
        "DeleteSkillVersionResponse", "ListSkillVersionsResponse", "ListSkillsParams",
        "ListSkillsResponse", "Skill", "SkillVersion",
    }),
    "anthropic_tool_search": frozenset({
        "TOOL_SEARCH_BETA_HEADER_ANTHROPIC", "TOOL_SEARCH_BETA_HEADER_BEDROCK",
        "TOOL_SEARCH_BETA_HEADER_BY_PROVIDER", "TOOL_SEARCH_BETA_HEADER_VERTEX",
        "get_tool_search_beta_header",
    }),
}
CONSTANTS: Final[dict[str, str | list[str]]] = {
    "ANTHROPIC_ADVISOR_TOOL_TYPE": "advisor_20260301",
    "ANTHROPIC_API_HEADERS": ["anthropic-beta", "anthropic-version"],
    "ANTHROPIC_API_ONLY_HEADERS": ["anthropic-beta"],
    "ANTHROPIC_EFFORT_BETA_HEADER": "effort-2025-11-24",
    "ANTHROPIC_FINE_GRAINED_TOOL_STREAMING_BETA_HEADER": "fine-grained-tool-streaming-2025-05-14",
    "ANTHROPIC_MID_CONVERSATION_OUTPUT_CONFIG_BETA_HEADER":
        "mid-conversation-output-config-2026-07-01",
    "ANTHROPIC_MID_CONVERSATION_TOOL_CHANGES_BETA_HEADER":
        "mid-conversation-tool-changes-2026-07-01",
    "ANTHROPIC_OAUTH_BETA_HEADER": "oauth-2025-04-20",
    "ANTHROPIC_OAUTH_TOKEN_PREFIX": "sk-ant-oat",
    "ANTHROPIC_PROMPT_CACHING_SCOPE_BETA_HEADER": "prompt-caching-scope-2026-01-05",
    "ANTHROPIC_THINKING_DISPLAY_UPDATES_BETA_HEADER": "thinking-display-updates-2026-08-18",
    "ANTHROPIC_TOOL_SEARCH_BETA_HEADER": "advanced-tool-use-2025-11-20",
    "ANTHROPIC_TOOL_SEARCH_TOOL_TYPES": [
        "tool_search_tool_bm25_20251119", "tool_search_tool_regex_20251119",
    ],
    "TOOL_SEARCH_BETA_HEADER_ANTHROPIC": "advanced-tool-use-2025-11-20",
    "TOOL_SEARCH_BETA_HEADER_BEDROCK": "tool-search-tool-2025-10-19",
    "TOOL_SEARCH_BETA_HEADER_VERTEX": "tool-search-tool-2025-10-19",
}
HOSTED_TOOL_MEMBERS: Final[dict[str, str]] = {
    "WEB_SEARCH": "web_search",
    "BASH": "bash",
    "TEXT_EDITOR": "text_editor",
    "CODE_EXECUTION": "code_execution",
    "WEB_FETCH": "web_fetch",
    "MEMORY": "memory",
    "TOOL_SEARCH_TOOL": "tool_search_tool",
}
BETA_HEADER_MEMBERS: Final[dict[str, str]] = {
    "WEB_FETCH_2025_09_10": "web-fetch-2025-09-10",
    "WEB_SEARCH_2025_03_05": "web-search-2025-03-05",
    "CONTEXT_MANAGEMENT_2025_06_27": "context-management-2025-06-27",
    "COMPACT_2026_01_12": "compact-2026-01-12",
    "COMPACT_2026_09_04": "compact-2026-09-04",
    "STRUCTURED_OUTPUT_2025_09_25": "structured-outputs-2025-11-13",
    "ADVANCED_TOOL_USE_2025_11_20": "advanced-tool-use-2025-11-20",
    "FAST_MODE_2026_02_01": "fast-mode-2026-02-01",
    "ADVISOR_TOOL_2026_03_01": "advisor-tool-2026-03-01",
    "PER_TURN_CONTROL_2026_07_01": "per-turn-control-2026-07-01",
    "DANGEROUS_TOOL_USE_2026_09_03": "dangerous-tool-use-2026-09-03",
}
TOOL_SEARCH_HEADERS: Final[dict[str, str]] = {
    "anthropic": "advanced-tool-use-2025-11-20",
    "azure": "advanced-tool-use-2025-11-20",
    "azure_ai": "advanced-tool-use-2025-11-20",
    "bedrock": "tool-search-tool-2025-10-19",
    "vertex_ai": "tool-search-tool-2025-10-19",
    "vertex_ai_beta": "tool-search-tool-2025-10-19",
}
TOOL_SEARCH_OUTPUTS: Final[dict[str, str]] = {
    "": "advanced-tool-use-2025-11-20",
    "anthropic": "advanced-tool-use-2025-11-20",
    "azure": "advanced-tool-use-2025-11-20",
    "azure_ai": "advanced-tool-use-2025-11-20",
    "bedrock": "tool-search-tool-2025-10-19",
    "openai": "advanced-tool-use-2025-11-20",
    "unknown": "advanced-tool-use-2025-11-20",
    "vertex_ai": "tool-search-tool-2025-10-19",
    "vertex_ai_beta": "tool-search-tool-2025-10-19",
}
RESPONSE_DUMP: Final[dict[str, JsonValue]] = {
    "content": [
        {"text": "hi", "type": "text"},
        {
            "id": "toolu_1",
            "input": {"city": "SF"},
            "name": "get_weather",
            "provider_specific_fields": {"region": "eu"},
            "type": "tool_use",
            "unknown_tool": 2,
        },
        {"signature": None, "thinking": "hm", "type": "thinking"},
        {"data": "opaque", "type": "redacted_thinking"},
    ],
    "id": "msg_1",
    "model": "claude-3",
    "role": "assistant",
    "stop_reason": "end_turn",
    "stop_sequence": None,
    "type": "message",
    "usage": {"input_tokens": 3, "output_tokens": 4, "unknown_usage": 5},
}
SKILL_DUMP: Final[dict[str, JsonValue]] = {
    "created_at": "2026-01-01T00:00:00Z",
    "description": None,
    "display_title": None,
    "id": "skill_1",
    "latest_version": None,
    "search_score": None,
    "source": "custom",
    "type": "skill",
    "updated_at": "2026-01-02T00:00:00Z",
}
SKILL_VERSION_DUMP: Final[dict[str, JsonValue]] = {
    "created_at": "2026-01-03T00:00:00Z",
    "description": None,
    "display_title": None,
    "id": "version_1",
    "instructions": None,
    "metadata": None,
    "skill_id": "skill_1",
    "type": "skill.version",
}
DESCRIBE_SCRIPT = """
import json
import sys
from enum import Enum
from types import ModuleType, UnionType
from typing import Literal, Union, get_args, get_origin, get_type_hints

from pydantic import BaseModel
from typing_extensions import is_typeddict

import betterlite.types.llms.anthropic as ours
import betterlite.types.llms.anthropic.anthropic_request as ours_request
import betterlite.types.llms.anthropic.anthropic_response as ours_response
import betterlite.types.llms.anthropic.anthropic_skills as ours_skills
import betterlite.types.llms.anthropic.anthropic_tool_search as ours_search
import litellm.types.llms.anthropic as theirs
import litellm.types.llms.anthropic_messages.anthropic_request as theirs_request
import litellm.types.llms.anthropic_messages.anthropic_response as theirs_response
import litellm.types.llms.anthropic_skills as theirs_skills
import litellm.types.llms.anthropic_tool_search as theirs_search

LIBRARY = {
    "anthropic": {
        "Any", "BaseModel", "ConfigDict", "Enum", "Final", "Iterable", "Literal",
        "NotRequired", "ReadOnly", "Required", "Sequence", "TypeAlias", "TypedDict",
    },
    "anthropic_request": {"BaseModel", "ClassVar", "ConfigDict", "ResponseObject"},
    "anthropic_response": {
        "Any", "JsonValue", "Literal", "NotRequired", "ReadOnly", "Sequence", "TypeAlias",
        "TypedDict",
    },
    "anthropic_skills": {
        "Any", "BaseModel", "ClassVar", "ConfigDict", "JsonValue", "ResponseObject", "TypedDict",
    },
    "anthropic_tool_search": {"Final", "LlmProviders", "Mapping", "MappingProxyType"},
}
GROUPS = {
    "anthropic": (theirs, ours),
    "anthropic_request": (theirs_request, ours_request),
    "anthropic_response": (theirs_response, ours_response),
    "anthropic_skills": (theirs_skills, ours_skills),
    "anthropic_tool_search": (theirs_search, ours_search),
}
TOOL_SEARCH_INPUTS = [
    "", "anthropic", "azure", "azure_ai", "bedrock", "openai", "unknown", "vertex_ai",
    "vertex_ai_beta",
]
EMPTY_SHAPE = {
    "kind": "",
    "keys": [],
    "required": [],
    "defaults": {},
    "literals": {},
    "members": [],
    "enum_members": [],
    "values": [],
}


def filled(shape):
    return EMPTY_SHAPE | shape


def literal_members(annotation):
    if get_origin(annotation) is Literal:
        return [
            json.dumps(member.value if isinstance(member, Enum) else member, sort_keys=True)
            for member in get_args(annotation)
        ]
    values = []
    if get_origin(annotation) is not None:
        for member in get_args(annotation):
            values.extend(literal_members(member))
    return values


def described_fields(annotations, names, required, defaults):
    literals = {}
    for name in names:
        annotation = annotations.get(name)
        if annotation is None:
            continue
        values = literal_members(annotation)
        if values:
            literals[name] = values
    return {
        "keys": sorted(names),
        "required": sorted(required),
        "defaults": defaults,
        "literals": literals,
    }


def typeddict_shape(value):
    described = described_fields(
        get_type_hints(value, include_extras=True),
        value.__required_keys__ | value.__optional_keys__,
        value.__required_keys__,
        {},
    )
    described["kind"] = "typeddict"
    return described


def model_shape(value):
    fields = value.model_fields
    annotations = {
        name: field.annotation
        for name, field in fields.items()
        if field.annotation is not None
    }
    described = described_fields(
        annotations,
        fields.keys(),
        [name for name, field in fields.items() if field.is_required()],
        {
            name: json.dumps(field.get_default(call_default_factory=False), sort_keys=True)
            for name, field in fields.items()
            if not field.is_required()
        },
    )
    described["kind"] = "model"
    return described


def member_label(member):
    if get_origin(member) is Literal:
        return "literal"
    name = getattr(member, "__name__", None)
    module = getattr(member, "__module__", "") or ""
    if module.startswith("openai") or get_origin(member) is not None:
        return "json"
    if name is None:
        return "literal"
    return name


def alias_members(value):
    labels = []
    for member in get_args(value):
        label = member_label(member)
        if not labels or labels[-1] != label:
            labels.append(label)
    return labels


def alias_shape(value):
    origin = get_origin(value)
    if origin is Union or origin is UnionType:
        return {"kind": "union", "members": alias_members(value)}
    if origin is list:
        return {"kind": "list"}
    return {"kind": "literal", "values": literal_members(value)}


def shape_of(value):
    if is_typeddict(value):
        return typeddict_shape(value)
    if isinstance(value, type) and issubclass(value, Enum):
        return {
            "kind": "enum",
            "enum_members": [[name, member.value] for name, member in value.__members__.items()],
        }
    if isinstance(value, type) and issubclass(value, BaseModel):
        return model_shape(value)
    return alias_shape(value)


def describe(module, names):
    return {
        name: None if not hasattr(module, name) else filled(shape_of(getattr(module, name)))
        for name in names
    }


def module_names(module, group):
    excluded = LIBRARY[group]
    return sorted(
        name
        for name in vars(module)
        if not name.startswith("_")
        and name not in excluded
        and not isinstance(getattr(module, name), ModuleType)
    )


def constant_values(module, names):
    found = {}
    for name in names:
        if not hasattr(module, name):
            continue
        value = getattr(module, name)
        if isinstance(value, str):
            found[name] = value
        elif isinstance(value, (set, frozenset)):
            found[name] = sorted(value)
    return found


def model_dumps(anthropic, skills, request):
    response = anthropic.AnthropicResponse(
        id="msg_1",
        type="message",
        role="assistant",
        content=[
            {"type": "text", "text": "hi", "unknown_text": 1},
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "get_weather",
                "input": {"city": "SF"},
                "provider_specific_fields": {"region": "eu"},
                "unknown_tool": 2,
            },
            {"type": "thinking", "thinking": "hm", "signature": None, "unknown_thinking": 3},
            {"type": "redacted_thinking", "data": "opaque", "unknown_redacted": 4},
        ],
        model="claude-3",
        stop_reason="end_turn",
        stop_sequence=None,
        usage={"input_tokens": 3, "output_tokens": 4, "unknown_usage": 5},
        unknown_response=6,
    )
    usage = anthropic.AnthropicResponseUsageBlock(
        input_tokens=7, output_tokens=8, unknown_usage_block=9,
    )
    tokens = anthropic.AnthropicOutputTokensDetails(thinking_tokens=10, unknown_tokens=11)
    metadata = request.AnthropicMetadata(user_id="user_1", unknown_metadata=12)
    skill = skills.Skill(
        id="skill_1",
        created_at="2026-01-01T00:00:00Z",
        source="custom",
        updated_at="2026-01-02T00:00:00Z",
        unknown_skill=13,
    )
    skills_list = skills.ListSkillsResponse(data=[skill], unknown_list=14)
    version = skills.SkillVersion(
        id="version_1", skill_id="skill_1", created_at="2026-01-03T00:00:00Z",
    )
    versions_list = skills.ListSkillVersionsResponse(data=[version], first_id="version_1")
    deleted_skill = skills.DeleteSkillResponse(id="skill_1")
    deleted_version = skills.DeleteSkillVersionResponse(id="version_1", deleted=True)
    return {
        "response": response.model_dump(mode="json"),
        "usage": usage.model_dump(mode="json"),
        "tokens": tokens.model_dump(mode="json"),
        "metadata": metadata.model_dump(mode="json"),
        "skill": skill.model_dump(mode="json"),
        "skills": skills_list.model_dump(mode="json"),
        "versions": versions_list.model_dump(mode="json"),
        "deleted_skill": deleted_skill.model_dump(mode="json"),
        "deleted_version": deleted_version.model_dump(mode="json"),
    }


def tool_search(module):
    return {
        "headers": dict(sorted(module.TOOL_SEARCH_BETA_HEADER_BY_PROVIDER.items())),
        "outputs": {key: module.get_tool_search_beta_header(key) for key in TOOL_SEARCH_INPUTS},
    }


if len(sys.argv) > 1:
    names = json.loads(sys.argv[1])
else:
    names = {group: module_names(GROUPS[group][0], group) for group in GROUPS}
print(
    json.dumps(
        {
            "shapes": {
                group: {
                    "litellm": describe(GROUPS[group][0], names[group]),
                    "betterlite": describe(GROUPS[group][1], names[group]),
                }
                for group in GROUPS
                if group != "anthropic_tool_search"
            },
            "names": {
                group: {
                    "litellm": module_names(GROUPS[group][0], group),
                    "betterlite": module_names(GROUPS[group][1], group),
                }
                for group in GROUPS
            },
            "constants": {
                "litellm": constant_values(theirs, names["anthropic"])
                | constant_values(theirs_search, names["anthropic_tool_search"]),
                "betterlite": constant_values(ours, names["anthropic"])
                | constant_values(ours_search, names["anthropic_tool_search"]),
            },
            "models": {
                "litellm": model_dumps(theirs, theirs_skills, theirs_request),
                "betterlite": model_dumps(ours, ours_skills, ours_request),
            },
            "tool_search": {
                "litellm": tool_search(theirs_search),
                "betterlite": tool_search(ours_search),
            },
        },
        sort_keys=True,
    )
)
"""


class Shape(TypedDict):
    kind: str
    keys: list[str]
    required: list[str]
    defaults: dict[str, str]
    literals: dict[str, list[str]]
    members: list[str]
    enum_members: list[tuple[str, str]]
    values: list[str]


class Sides(TypedDict):
    litellm: dict[str, Shape | None]
    betterlite: dict[str, Shape | None]


class Shapes(TypedDict):
    anthropic: Sides
    anthropic_request: Sides
    anthropic_response: Sides
    anthropic_skills: Sides


class Names(TypedDict):
    litellm: list[str]
    betterlite: list[str]


class Constants(TypedDict):
    litellm: dict[str, str | list[str]]
    betterlite: dict[str, str | list[str]]


class ModelDumps(TypedDict):
    response: dict[str, JsonValue]
    usage: dict[str, JsonValue]
    tokens: dict[str, JsonValue]
    metadata: dict[str, JsonValue]
    skill: dict[str, JsonValue]
    skills: dict[str, JsonValue]
    versions: dict[str, JsonValue]
    deleted_skill: dict[str, JsonValue]
    deleted_version: dict[str, JsonValue]


class Models(TypedDict):
    litellm: ModelDumps
    betterlite: ModelDumps


class ToolSearchSide(TypedDict):
    headers: dict[str, str]
    outputs: dict[str, str]


class ToolSearch(TypedDict):
    litellm: ToolSearchSide
    betterlite: ToolSearchSide


class Report(TypedDict):
    shapes: Shapes
    names: dict[str, Names]
    constants: Constants
    models: Models
    tool_search: ToolSearch


@pytest.fixture(scope="module")
def report() -> Report:
    requested: dict[str, list[str]] = {group: sorted(NAMES[group]) for group in NAMES}
    names = json.dumps(requested)
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT, names],
        check=True,
        capture_output=True,
        text=True,
    )
    parsed: Report = TypeAdapter(Report).validate_json(completed.stdout)
    return parsed


def check_sides(sides: Sides, names: frozenset[str]) -> None:
    assert set(sides["litellm"]) == names
    assert set(sides["betterlite"]) == names
    assert None not in sides["litellm"].values()
    assert None not in sides["betterlite"].values()
    assert sides["betterlite"] == sides["litellm"]


def test_names_shapes(report: Report) -> None:
    shapes = report["shapes"]
    check_sides(shapes["anthropic"], NAMES["anthropic"])
    check_sides(shapes["anthropic_request"], NAMES["anthropic_request"])
    check_sides(shapes["anthropic_response"], NAMES["anthropic_response"])
    check_sides(shapes["anthropic_skills"], NAMES["anthropic_skills"])
    for group, sides in report["names"].items():
        expected = sorted(NAMES[group])
        assert sides["litellm"] == expected
        assert sides["betterlite"] == expected


def test_names_requests(report: Report) -> None:
    tools = report["shapes"]["anthropic"]["betterlite"]
    request = tools["AnthropicMessagesRequest"]
    assert request is not None
    assert request["required"] == ["messages", "model"]
    assert "compaction" in request["keys"]
    assert "litellm_metadata" in request["keys"]
    assert "safeguards" in request["keys"]
    optional = tools["AnthropicMessagesRequestOptionalParams"]
    assert optional is not None
    assert optional["required"] == []
    tool = tools["AnthropicMessagesTool"]
    assert tool is not None
    assert tool["required"] == ["name"]
    assert tool["literals"] == {"type": ['"custom"']}
    choice = tools["AnthropicMessagesToolChoice"]
    assert choice is not None
    assert choice["required"] == ["type"]
    assert choice["literals"] == {"type": ['"auto"', '"any"', '"tool"', '"none"']}
    compaction = tools["AnthropicCompaction"]
    assert compaction is not None
    assert compaction["required"] == ["type"]
    assert compaction["literals"] == {"type": ['"summarize"']}
    schema = tools["AnthropicInputSchema"]
    assert schema is not None
    assert schema["keys"] == [
        "$defs", "additionalProperties", "properties", "required", "strict", "type",
    ]
    assert schema["required"] == []
    output = tools["AnthropicOutputConfig"]
    assert output is not None
    assert output["literals"] == {
        "effort": ['"high"', '"medium"', '"low"', '"xhigh"', '"max"'],
    }
    finish = tools["AnthropicFinishReason"]
    assert finish is not None
    assert finish["kind"] == "literal"
    assert finish["values"] == [
        '"end_turn"', '"max_tokens"', '"stop_sequence"', '"tool_use"', '"refusal"',
    ]
    delta = tools["StreamingContentBlockDeltaType"]
    assert delta is not None
    assert delta["values"] == [
        '"text_delta"', '"input_json_delta"', '"thinking_delta"', '"signature_delta"',
    ]


def test_names_tools(report: Report) -> None:
    tools = report["shapes"]["anthropic"]["betterlite"]
    hosted = tools["AnthropicHostedTools"]
    assert hosted is not None
    assert hosted["required"] == ["name", "type"]
    web_search = tools["AnthropicWebSearchTool"]
    assert web_search is not None
    assert web_search["literals"] == {"name": ['"web_search"']}
    advisor = tools["AnthropicAdvisorTool"]
    assert advisor is not None
    assert advisor["literals"] == {"name": ['"advisor"'], "type": ['"advisor_20260301"']}
    citation = tools["AnthropicCitation"]
    assert citation is not None
    assert citation["members"] == [
        "AnthropicCitationPageLocation", "AnthropicCitationCharLocation",
    ]
    all_tools = tools["AllAnthropicToolsValues"]
    assert all_tools is not None
    assert all_tools["members"] == [
        "AnthropicComputerTool", "AnthropicHostedTools", "AnthropicMessagesTool",
        "AnthropicWebSearchTool", "AnthropicCodeExecutionTool", "AnthropicMemoryTool",
        "AnthropicToolSearchToolRegex", "AnthropicToolSearchToolBM25", "AnthropicAdvisorTool",
    ]
    caller = tools["ToolCaller"]
    assert caller is not None
    assert caller["members"] == ["DirectToolCaller", "CodeExecutionToolCaller"]
    assistant = tools["AnthopicMessagesAssistantMessageParam"]
    assert assistant is not None
    assert assistant["required"] == ["content", "role"]
    assert assistant["literals"] == {"role": ['"assistant"']}
    document = tools["AnthropicMessagesDocumentParam"]
    assert document is not None
    assert document["required"] == ["source", "type"]
    metadata = tools["AnthropicMetadata"]
    assert metadata is not None
    assert metadata["kind"] == "typeddict"
    assert metadata["keys"] == ["user_id"]


def test_names_blocks(report: Report) -> None:
    response = report["shapes"]["anthropic_response"]["betterlite"]
    messages = response["AnthropicMessagesResponse"]
    assert messages is not None
    assert messages["literals"]["stop_reason"] == [
        '"end_turn"', '"max_tokens"', '"stop_sequence"', '"tool_use"', '"refusal"',
        '"compaction"',
    ]
    usage = response["AnthropicUsage"]
    assert usage is not None
    assert usage["keys"] == [
        "cache_creation_input_tokens", "cache_read_input_tokens", "input_tokens",
        "iterations", "output_tokens", "server_tool_use",
    ]
    blocks = response["AnthropicResponseContentBlock"]
    assert blocks is not None
    assert blocks["members"] == [
        "AnthropicResponseTextBlock", "AnthropicResponseToolUseBlock",
        "AnthropicResponseThinkingBlock", "AnthropicResponseRedactedThinkingBlock",
        "CompactionBlock",
    ]
    iteration = response["UsageIteration"]
    assert iteration is not None
    assert iteration["required"] == ["type"]
    assert iteration["literals"] == {"type": ['"compaction"', '"message"']}
    block = response["CompactionBlock"]
    assert block is not None
    assert block["required"] == ["type"]
    assert block["literals"] == {"type": ['"compaction"']}
    metadata = report["shapes"]["anthropic_request"]["betterlite"]["AnthropicMetadata"]
    assert metadata is not None
    assert metadata["kind"] == "model"
    assert metadata["defaults"] == {"user_id": "null"}


def test_names_constants(report: Report) -> None:
    constants = report["constants"]
    assert constants["betterlite"] == constants["litellm"]
    assert constants["betterlite"] == CONSTANTS
    assert sorted(ANTHROPIC_API_HEADERS) == ["anthropic-beta", "anthropic-version"]
    assert sorted(ANTHROPIC_API_ONLY_HEADERS) == ["anthropic-beta"]


def test_names_enums(report: Report) -> None:
    shapes = report["shapes"]["anthropic"]["betterlite"]
    hosted = shapes["ANTHROPIC_HOSTED_TOOLS"]
    assert hosted is not None
    assert hosted["kind"] == "enum"
    assert dict(hosted["enum_members"]) == HOSTED_TOOL_MEMBERS
    beta = shapes["ANTHROPIC_BETA_HEADER_VALUES"]
    assert beta is not None
    assert beta["kind"] == "enum"
    assert dict(beta["enum_members"]) == BETA_HEADER_MEMBERS
    assert AnthropicHostedToolNames.BASH.value == "bash"
    assert AnthropicHostedToolNames.TOOL_SEARCH_TOOL.value == "tool_search_tool"
    assert AnthropicBetaHeaderValues.COMPACT_2026_01_12.value == "compact-2026-01-12"
    assert AnthropicBetaHeaderValues.WEB_FETCH_2025_09_10.value == "web-fetch-2025-09-10"


def test_names_tool_search(report: Report) -> None:
    sides = report["tool_search"]
    assert sides["betterlite"] == sides["litellm"]
    assert sides["betterlite"]["headers"] == TOOL_SEARCH_HEADERS
    assert sides["betterlite"]["outputs"] == TOOL_SEARCH_OUTPUTS
    assert get_tool_search_beta_header("vertex_ai") == "tool-search-tool-2025-10-19"
    assert get_tool_search_beta_header("bedrock") == "tool-search-tool-2025-10-19"
    assert get_tool_search_beta_header("openai") == "advanced-tool-use-2025-11-20"
    assert get_tool_search_beta_header("") == "advanced-tool-use-2025-11-20"


def test_names_models(report: Report) -> None:
    models = report["models"]
    assert models["betterlite"] == models["litellm"]
    dumps = models["betterlite"]
    assert dumps["response"] == RESPONSE_DUMP
    assert dumps["usage"] == {
        "input_tokens": 7, "output_tokens": 8, "unknown_usage_block": 9,
    }
    assert dumps["tokens"] == {"thinking_tokens": 10, "unknown_tokens": 11}
    assert dumps["metadata"] == {"user_id": "user_1"}


def test_names_skills(report: Report) -> None:
    shapes = report["shapes"]["anthropic_skills"]["betterlite"]
    skill = shapes["Skill"]
    assert skill is not None
    assert skill["kind"] == "model"
    assert skill["required"] == ["created_at", "id", "source", "updated_at"]
    assert skill["defaults"] == {
        "description": "null",
        "display_title": "null",
        "latest_version": "null",
        "search_score": "null",
        "type": '"skill"',
    }
    version = shapes["SkillVersion"]
    assert version is not None
    assert version["required"] == ["created_at", "id", "skill_id"]
    assert version["defaults"]["type"] == '"skill.version"'
    versions = shapes["ListSkillVersionsResponse"]
    assert versions is not None
    assert versions["required"] == ["data"]
    assert versions["defaults"] == {
        "first_id": "null", "has_more": "false", "last_id": "null", "object": '"list"',
    }
    deleted = shapes["DeleteSkillVersionResponse"]
    assert deleted is not None
    assert deleted["required"] == ["deleted", "id"]
    assert deleted["defaults"] == {"object": '"skill.version.deleted"'}
    assert shapes["CreateSkillRequest"] is not None
    assert shapes["ListSkillsParams"] is not None
    dumps = report["models"]["betterlite"]
    assert dumps["skill"] == SKILL_DUMP
    assert dumps["skills"] == {
        "data": [SKILL_DUMP],
        "has_more": False,
        "next_page": None,
    }
    assert dumps["versions"] == {
        "data": [SKILL_VERSION_DUMP],
        "first_id": "version_1",
        "has_more": False,
        "last_id": None,
        "object": "list",
    }
    assert dumps["deleted_skill"] == {"id": "skill_1", "type": "skill_deleted"}
    assert dumps["deleted_version"] == {
        "deleted": True, "id": "version_1", "object": "skill.version.deleted",
    }
