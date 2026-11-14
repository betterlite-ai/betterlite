import json
import subprocess
import sys
from typing import Final

import pytest
from pydantic import JsonValue, TypeAdapter

LITELLM: Final = "litellm.types"
BETTERLITE: Final = "betterlite.types"

PROBE: Final = """
import dataclasses
import importlib
import inspect
import json
import sys
import types
import typing

from pydantic import BaseModel, TypeAdapter, ValidationError
from pydantic_core import to_jsonable_python

prefix, mode, spec = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])


def load(name):
    return importlib.import_module(prefix + "." + name)


def named(value):
    if inspect.isclass(value):
        return value.__name__
    return repr(value)


def plain(value):
    return json.loads(json.dumps(to_jsonable_python(value, fallback=named)))


def shape(tp):
    origin = typing.get_origin(tp)
    args = typing.get_args(tp)
    if origin is None:
        if tp is None or tp is type(None):
            return "None"
        name = getattr(tp, "__name__", None)
        if name is None:
            return repr(tp).replace("betterlite.types.", "").replace("litellm.types.", "")
        if name == "object":
            return "JsonValue"
        return name
    name = getattr(origin, "__name__", repr(origin))
    if name == "Literal":
        return "Literal" + repr([list(args)])
    if name in ("Union", "UnionType"):
        return "|".join(sorted(shape(item) for item in args))
    if name == "Annotated":
        return shape(args[0])
    return name + "[" + ",".join(shape(item) for item in args) + "]"


def field_view(info):
    entry = {
        "required": info.is_required(),
        "shape": shape(info.annotation),
        "constraints": sorted(repr(item) for item in info.metadata),
    }
    if not info.is_required():
        entry["default"] = plain(info.get_default(call_default_factory=True))
    return entry


def model_fields(obj, skipped):
    found = {}
    for name, info in obj.model_fields.items():
        entry = field_view(info)
        if name in skipped:
            entry["shape"] = skipped[name]
        found[name] = entry
    return found


def dataclass_fields(obj):
    found = []
    for item in dataclasses.fields(obj):
        entry = {"name": item.name, "shape": shape(item.type), "kw_only": item.kw_only}
        if item.default_factory is not dataclasses.MISSING:
            entry["factory"] = plain(item.default_factory())
        elif item.default is not dataclasses.MISSING:
            entry["default"] = plain(item.default)
        found.append(entry)
    return found


def describe(obj, skipped):
    origin = typing.get_origin(obj)
    if hasattr(obj, "__required_keys__"):
        hints = typing.get_type_hints(obj)
        return {
            "kind": "typeddict",
            "required": sorted(obj.__required_keys__),
            "optional": sorted(obj.__optional_keys__),
            "readonly": sorted(obj.__readonly_keys__),
            "total": obj.__total__,
            "hints": {
                key: skipped.get(key, shape(value)) for key, value in hints.items()
            },
        }
    if inspect.isclass(obj) and issubclass(obj, BaseModel):
        return {
            "kind": "model",
            "bases": [base.__name__ for base in obj.__bases__],
            "extra": obj.model_config.get("extra"),
            "frozen": obj.model_config.get("frozen"),
            "fields": model_fields(obj, skipped),
        }
    if inspect.isclass(obj) and dataclasses.is_dataclass(obj):
        return {
            "kind": "dataclass",
            "frozen": obj.__dataclass_params__.frozen,
            "slots": hasattr(obj, "__slots__"),
            "fields": dataclass_fields(obj),
        }
    if origin is typing.Literal:
        return {"kind": "literal", "values": [list(typing.get_args(obj))]}
    if origin is typing.Annotated:
        base, *extras = typing.get_args(obj)
        discriminators = [getattr(item, "discriminator", None) for item in extras]
        return {"kind": "annotated", "base": shape(base), "discriminators": discriminators}
    if origin is typing.Union or origin is types.UnionType:
        return {"kind": "union", "members": [shape(item) for item in typing.get_args(obj)]}
    if callable(obj) and not inspect.isclass(obj):
        return {"kind": "callable"}
    return {"kind": "value", "value": plain(obj)}


def place(loc):
    return [
        part
        for part in loc
        if isinstance(part, int) or (isinstance(part, str) and part.isidentifier())
    ]


def outcome(cls, payload):
    try:
        value = cls.model_validate(payload)
    except ValidationError as error:
        places = [[item["type"], place(item["loc"])] for item in error.errors(include_url=False)]
        return {"error": places}
    return {"dump": plain(value.model_dump(mode="json"))}


def special_agents(module, spec):
    succeeded = [
        module.AgentKillSwitchResult(
            agent_id="a", url="u", method="POST", status_code=status,
        ).succeeded
        for status in spec["statuses"]
    ]
    return {
        "succeeded": succeeded,
        "headers": [module.AGENT_CALLER_USER_ID_HEADER, module.AGENT_CALLER_TEAM_ID_HEADER],
    }


def special_decisions(module, spec):
    adapter = TypeAdapter(module.DecisionsCallParams)
    response = module.DecisionsResponse(answers={"a": {"type": "noul", "noul": 1.0}})
    response.set_hidden_params({"request_id": "r"})
    question = module.NoulQuestion(type="noul", instructions="q")
    frozen = []
    try:
        question.instructions = "other"
        frozen.append("assigned")
    except ValidationError:
        frozen.append("rejected")
    return {
        "call_params": plain(adapter.validate_python(spec["call_params"])),
        "choice_keys": [module.systemone_choice_key(item) for item in spec["choice_keys"]],
        "hidden": [plain(response.hidden_params), plain(response.model_dump(mode="json"))],
        "usage": plain(module.DecisionsUsage().model_dump(mode="json")),
        "frozen": frozen,
        "max_questions": module.MAX_DECISION_QUESTIONS,
    }


if mode == "describe":
    result = {}
    for module_name, names in spec["names"].items():
        module = load(module_name)
        skipped = spec["skip_shapes"].get(module_name, {})
        result[module_name] = {
            name: describe(getattr(module, name), skipped.get(name, {}))
            if hasattr(module, name)
            else {"kind": "missing"}
            for name in names
        }
elif mode == "defined":
    result = {}
    for module_name in spec["modules"]:
        module = load(module_name)
        result[module_name] = sorted(
            name
            for name, item in vars(module).items()
            if (inspect.isclass(item) or inspect.isfunction(item))
            and item.__module__ == module.__name__
        )
elif mode == "behavior":
    outcomes = []
    for case in spec["cases"]:
        module = load(case["module"])
        outcomes.append(outcome(getattr(module, case["name"]), case["payload"]))
    result = {"cases": outcomes}
elif mode == "special" and spec["module"] == "agents":
    result = special_agents(load("agents"), spec)
elif mode == "special" and spec["module"] == "decisions":
    result = special_decisions(load("decisions"), spec)
else:
    raise SystemExit("unknown probe mode")
print(json.dumps(result))
"""

AGENT_NAMES: Final = (
    "AgentProvider",
    "AgentExtension",
    "AgentCapabilities",
    "SecuritySchemeBase",
    "APIKeySecurityScheme",
    "HTTPAuthSecurityScheme",
    "MutualTLSSecurityScheme",
    "OAuthFlows",
    "OAuth2SecurityScheme",
    "OpenIdConnectSecurityScheme",
    "SecurityScheme",
    "AgentSkill",
    "AgentInterface",
    "AgentCardSignature",
    "AgentCard",
    "AugmentedAgentCard",
    "AgentObjectPermission",
    "AgentKillSwitchBearerAuth",
    "AgentKillSwitchApiKeyAuth",
    "AgentKillSwitchBasicAuth",
    "AgentKillSwitchAuth",
    "AgentKillSwitchMethod",
    "AgentKillSwitchConfig",
    "AgentKillSwitchResult",
    "AgentConfig",
    "PatchAgentRequest",
    "AGENT_CALLER_USER_ID_HEADER",
    "AGENT_CALLER_TEAM_ID_HEADER",
    "AgentCaller",
    "AgentKeySummary",
    "AgentResponse",
    "ListAgentsResponse",
    "AgentCreateResponse",
    "AgentDeleteResult",
    "AgentListResponse",
    "AgentVersionsResponse",
    "AgentMakePublicResponse",
    "MakeAgentsPublicRequest",
)

DECISION_NAMES: Final = (
    "DecisionsJSON",
    "NoulCriteria",
    "MAX_DECISION_QUESTIONS",
    "NoulQuestion",
    "ChoiceQuestion",
    "ScoreQuestion",
    "DecisionQuestion",
    "DecisionQuestionMap",
    "DecisionsRequestBody",
    "DecisionsRequest",
    "DecisionsCallParams",
    "NoulAnswer",
    "ChoiceAnswer",
    "ScoreAnswer",
    "DecisionAnswer",
    "DecisionsUsage",
    "_HiddenParamsResponse",
    "DecisionsResponse",
    "OpenAIDecisionInputText",
    "OpenAIDecisionInputImage",
    "OpenAIDecisionContentPart",
    "OpenAIDecisionInputMessage",
    "OpenAIDecisionInput",
    "OpenAIPredicateQuestion",
    "OpenAIChoiceOption",
    "systemone_choice_key",
    "OpenAIChoiceQuestion",
    "OpenAIScoreLevel",
    "OpenAIScoreQuestion",
    "OpenAIDecisionQuestion",
    "OpenAIDecisionRequestBody",
    "OpenAIDecisionRequest",
    "OpenAIPredicateAnswer",
    "OpenAIChoiceProbability",
    "OpenAIChoiceAnswer",
    "OpenAIScoreProbability",
    "OpenAIScoreAnswer",
    "OpenAIRefusalAnswer",
    "OpenAIDecisionAnswer",
    "OpenAIDecisionInputTokensDetails",
    "OpenAIDecisionOutputTokensDetails",
    "OpenAIDecisionUsage",
    "OpenAIDecisionResponse",
)

ACCESS_NAMES: Final = (
    "AccessGroupCreateRequest",
    "AccessGroupUpdateRequest",
    "AccessGroupResource",
    "AccessGroupResponse",
)

MODULES: Final[dict[str, tuple[str, ...]]] = {
    "agents": AGENT_NAMES,
    "decisions": DECISION_NAMES,
    "access_group": ACCESS_NAMES,
}

# docs/deviations/B7942.yaml: identity is owned by B7949 and agent_card_params holds Any
DOCUMENTED_SHAPES: Final = {
    "agents": {
        "AgentConfig": {"identity": "documented"},
        "PatchAgentRequest": {"identity": "documented"},
        "AgentResponse": {"identity": "documented", "agent_card_params": "documented"},
    },
}

DESCRIBE_SPEC: Final = json.dumps({"names": MODULES, "skip_shapes": DOCUMENTED_SHAPES})

REFUSAL = 30
AGENT_STATUSES: Final = [None, 199, 200, 299, 300]

CALL_PARAMS: Final = {
    "model": "m",
    "state": "s",
    "questions": {"q": {"type": "noul", "instructions": "x"}},
    "extra_headers": {"a": "b"},
    "unknown": 1,
}

AGENT_SPECIAL_SPEC: Final = json.dumps(
    {"module": "agents", "statuses": AGENT_STATUSES},
)
DECISION_SPECIAL_SPEC: Final = json.dumps(
    {
        "module": "decisions",
        "call_params": CALL_PARAMS,
        "choice_keys": [True, False, "true"],
    },
)

CASES: Final[tuple[tuple[str, str, JsonValue], ...]] = (
    ("decisions", "NoulQuestion", {"type": "noul"}),
    ("decisions", "NoulQuestion", {"type": "noul", "instructions": "is it open?"}),
    ("decisions", "NoulQuestion", {"type": "noul", "criteria": {"true": "open", "false": None}}),
    ("decisions", "NoulQuestion", {"type": "noul", "criteria": {"maybe": "x"}}),
    ("decisions", "NoulQuestion", {"type": "choice", "criteria": {"a": None}}),
    ("decisions", "NoulQuestion", {"type": "noul", "instructions": None, "extra": 3}),
    ("decisions", "NoulQuestion", "text"),
    ("decisions", "ChoiceQuestion", {"type": "choice", "criteria": {"a": None, "b": "x"}}),
    ("decisions", "ChoiceQuestion", {"type": "choice", "criteria": {}}),
    ("decisions", "ChoiceQuestion", {"type": "choice", "criteria": {"a": None}, "x": [1]}),
    ("decisions", "ScoreQuestion", {"type": "score", "criteria": ["a", "b"]}),
    ("decisions", "ScoreQuestion", {"type": "score", "criteria": []}),
    ("decisions", "ScoreQuestion", {"type": "score", "criteria": list(range(10))}),
    ("decisions", "ScoreQuestion", {"type": "score", "criteria": list(range(11))}),
    (
        "decisions",
        "DecisionsRequestBody",
        {"state": "ticket", "questions": {"ok": {"type": "noul", "instructions": "fine?"}}},
    ),
    (
        "decisions",
        "DecisionsRequestBody",
        {
            "state": {"a": [1, "b", None]},
            "questions": {"q": {"type": "score", "criteria": ["low", {"x": 1}]}},
        },
    ),
    ("decisions", "DecisionsRequestBody", {"state": "x", "questions": {}}),
    (
        "decisions",
        "DecisionsRequestBody",
        {"state": "x", "questions": {"": {"type": "noul", "instructions": "q"}}},
    ),
    (
        "decisions",
        "DecisionsRequest",
        {"state": "x", "questions": {"ok": {"type": "noul", "instructions": "q"}}},
    ),
    (
        "decisions",
        "DecisionsRequest",
        {"state": "x", "questions": {"ok": {"type": "noul", "instructions": "q"}}, "model": "m"},
    ),
    ("decisions", "DecisionsUsage", {}),
    ("decisions", "DecisionsUsage", {"input_tokens": 5, "output_tokens": 7, "cached_tokens": 2}),
    (
        "decisions",
        "DecisionsResponse",
        {
            "model": "m",
            "answers": {
                "ok": {"type": "noul", "noul": 0.9},
                "pick": {
                    "type": "choice",
                    "choice": "a",
                    "confidence": 0.5,
                    "probabilities": {"a": 0.5, "b": 0.5},
                },
            },
            "usage": {"input_tokens": 1, "output_tokens": 2},
        },
    ),
    ("decisions", "DecisionsResponse", {"answers": {"bad": {"type": "noul"}}}),
    ("decisions", "OpenAIDecisionInputText", {"type": "input_text", "text": "hi"}),
    ("decisions", "OpenAIDecisionInputImage", {"type": "input_image", "image_url": "u"}),
    (
        "decisions",
        "OpenAIDecisionInputMessage",
        {"content": [{"type": "input_text", "text": "a"}]},
    ),
    ("decisions", "OpenAIDecisionInputMessage", {"content": "x", "role": "assistant"}),
    (
        "decisions",
        "OpenAIPredicateQuestion",
        {"type": "predicate", "name": "spam", "instructions": "is it spam?"},
    ),
    (
        "decisions",
        "OpenAIChoiceQuestion",
        {"type": "choice", "instructions": "which?", "choices": [{"value": "a"}]},
    ),
    (
        "decisions",
        "OpenAIChoiceQuestion",
        {
            "type": "choice",
            "instructions": "which?",
            "choices": [{"value": "a"}, {"value": True}],
        },
    ),
    (
        "decisions",
        "OpenAIChoiceQuestion",
        {
            "type": "choice",
            "instructions": "which?",
            "choices": [{"value": "true"}, {"value": True}],
        },
    ),
    (
        "decisions",
        "OpenAIChoiceQuestion",
        {
            "type": "choice",
            "instructions": "which?",
            "choices": [{"value": "a"}, {"value": "b"}, {"value": "a"}],
        },
    ),
    (
        "decisions",
        "OpenAIScoreQuestion",
        {
            "type": "score",
            "instructions": "how bad?",
            "levels": [{"label": "low"}, {"label": "high", "description": "very"}],
        },
    ),
    (
        "decisions",
        "OpenAIScoreQuestion",
        {"type": "score", "instructions": "how bad?", "levels": [{"label": "low"}]},
    ),
    (
        "decisions",
        "OpenAIDecisionRequestBody",
        {"input": "hello", "questions": [{"type": "predicate", "instructions": "rude?"}]},
    ),
    (
        "decisions",
        "OpenAIDecisionRequest",
        {"input": "hello", "questions": [{"type": "predicate", "instructions": "rude?"}]},
    ),
    (
        "decisions",
        "OpenAIDecisionRequest",
        {
            "input": "hello",
            "questions": [{"type": "predicate", "instructions": "rude?"}],
            "model": "m",
            "safety_identifier": "s",
        },
    ),
    ("decisions", "OpenAIDecisionRequestBody", {"input": "x", "questions": []}),
    (
        "decisions",
        "OpenAIDecisionRequestBody",
        {
            "input": [{"content": "hi"}],
            "questions": [{"type": "predicate", "instructions": "q"}],
        },
    ),
    (
        "decisions",
        "OpenAIDecisionRequestBody",
        {"input": "x", "questions": [{"type": "predicate", "instructions": "q", "e": 1}]},
    ),
    (
        "decisions",
        "OpenAIDecisionResponse",
        {
            "model": "m",
            "answers": [
                {"type": "predicate", "name": None, "probability": 0.25},
                {
                    "type": "choice",
                    "name": "n",
                    "choice": True,
                    "probabilities": [{"value": "a", "probability": 0.5}],
                    "confidence": 0.5,
                },
                {"type": "refusal", "name": None},
            ],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "total_tokens": 3,
                "input_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 5},
                "output_tokens_details": {"reasoning_tokens": 6},
            },
        },
    ),
    (
        "decisions",
        "OpenAIDecisionUsage",
        {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3},
    ),
    ("decisions", "OpenAIDecisionUsage", {"input_tokens": 1, "output_tokens": 2}),
    ("decisions", "OpenAIRefusalAnswer", {"type": "refusal", "name": "n"}),
    ("decisions", "OpenAIRefusalAnswer", {"name": "n"}),
    ("agents", "AgentKillSwitchConfig", {"url": "https://example.com/hook"}),
    (
        "agents",
        "AgentKillSwitchConfig",
        {
            "url": "http://example.com/hook",
            "method": "PUT",
            "headers": {"a": "b"},
            "query_params": {"q": "1"},
            "body": {"x": [1]},
            "auth": {"type": "bearer", "token": "t"},
        },
    ),
    (
        "agents",
        "AgentKillSwitchConfig",
        {"url": "https://example.com/hook", "auth": {"type": "api_key", "api_key": "k"}},
    ),
    (
        "agents",
        "AgentKillSwitchConfig",
        {
            "url": "https://example.com/hook",
            "auth": {"type": "basic", "username": "u", "password": "p"},
        },
    ),
    ("agents", "AgentKillSwitchConfig", {"url": "ftp://example.com/hook"}),
    ("agents", "AgentKillSwitchConfig", {"url": "https://"}),
    ("agents", "AgentKillSwitchConfig", {"url": "example.com"}),
    ("agents", "AgentKillSwitchConfig", {"url": "https://example.com", "method": "HEAD"}),
    ("agents", "AgentKillSwitchConfig", {"url": "https://example.com", "extra": 1}),
    (
        "agents",
        "AgentKillSwitchResult",
        {"agent_id": "a", "url": "https://example.com", "method": "POST", "status_code": 204},
    ),
    (
        "agents",
        "AgentKillSwitchResult",
        {"agent_id": "a", "url": "https://example.com", "method": "POST", "error": "boom"},
    ),
    ("agents", "AgentCaller", {}),
    ("agents", "AgentCaller", {"user_id": "u", "team_id": "t", "extra": 1}),
    ("agents", "AgentKeySummary", {"token": "t"}),
    (
        "agents",
        "AgentResponse",
        {"agent_id": "a", "agent_name": "n", "agent_card_params": {"name": "n", "skills": []}},
    ),
    ("agents", "AgentResponse", {"agent_name": "n", "agent_card_params": {}}),
    (
        "agents",
        "AgentResponse",
        {
            "agent_id": "a",
            "agent_name": "n",
            "agent_card_params": {"name": "n"},
            "execution_mode": "delegated",
            "enabled": False,
            "spend": 1.5,
            "access_group_ids": ["g"],
            "keys": [{"token": "t", "key_alias": "a"}],
            "created_at": "2026-01-01T00:00:00Z",
            "kill_switch": {"url": "https://example.com", "auth": {"type": "bearer", "token": "t"}},
            "search_score": 0.5,
        },
    ),
    (
        "agents",
        "ListAgentsResponse",
        {"agents": [{"agent_id": "a", "agent_name": "n", "agent_card_params": {}}]},
    ),
    ("agents", "AgentCreateResponse", {"id": "i"}),
    ("agents", "AgentCreateResponse", {"id": "i", "extra": {"a": 1}}),
    ("agents", "AgentDeleteResult", {"name": "n"}),
    ("agents", "AgentListResponse", {}),
    ("agents", "AgentListResponse", {"agents": [{"id": "a"}], "next_page_token": "t", "x": 1}),
    ("agents", "AgentVersionsResponse", {}),
    ("agents", "AgentVersionsResponse", {"agent_versions": [{"name": "v"}]}),
    (
        "agents",
        "AgentMakePublicResponse",
        {"message": "m", "public_agent_groups": ["g"], "updated_by": "u"},
    ),
    ("agents", "MakeAgentsPublicRequest", {"agent_ids": ["a"]}),
    (
        "access_group",
        "AccessGroupCreateRequest",
        {"access_group_name": "n", "access_model_names": ["m"], "extra": 1},
    ),
    ("access_group", "AccessGroupUpdateRequest", {}),
    ("access_group", "AccessGroupResource", {"id": "i", "name": None}),
    (
        "access_group",
        "AccessGroupResponse",
        {
            "access_group_id": "g",
            "access_group_name": "n",
            "access_model_names": ["m"],
            "access_mcp_server_ids": [],
            "access_agent_ids": [],
            "assigned_team_ids": [],
            "assigned_key_ids": [],
            "access_mcp_servers": [{"id": "s", "name": None}],
            "access_agents": [],
            "assigned_teams": [{"id": "t", "name": "team"}],
            "assigned_keys": [],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
        },
    ),
    ("access_group", "AccessGroupResponse", {"access_group_id": "g"}),
)

BEHAVIOR_SPEC: Final = json.dumps(
    {
        "cases": [
            {"module": module, "name": name, "payload": payload}
            for module, name, payload in CASES
        ],
    },
)

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
LIST: Final = TypeAdapter(list[JsonValue])


def run_probe(prefix: str, mode: str, spec: str) -> dict[str, JsonValue]:
    done = subprocess.run(
        [sys.executable, "-c", PROBE, prefix, mode, spec],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return OBJECT.validate_json(done.stdout)


def described(prefix: str, mode: str, spec: str) -> dict[str, dict[str, JsonValue]]:
    result: dict[str, dict[str, JsonValue]] = {}
    for module, view in run_probe(prefix, mode, spec).items():
        result[module] = OBJECT.validate_python(view)
    return result


@pytest.fixture(scope="module")
def views() -> dict[str, dict[str, dict[str, JsonValue]]]:
    return {
        "theirs": described(LITELLM, "describe", DESCRIBE_SPEC),
        "ours": described(BETTERLITE, "describe", DESCRIBE_SPEC),
    }


@pytest.mark.parametrize("module", MODULES)
def test_names_listed(
    module: str,
    views: dict[str, dict[str, dict[str, JsonValue]]],
) -> None:
    ours = views["ours"][module]
    theirs = views["theirs"][module]
    assert set(ours) == set(MODULES[module])
    assert all(item != {"kind": "missing"} for item in theirs.values())
    assert all(item != {"kind": "missing"} for item in ours.values())


@pytest.mark.parametrize("module", MODULES)
def test_names_match_litellm(
    module: str,
    views: dict[str, dict[str, dict[str, JsonValue]]],
) -> None:
    theirs = views["theirs"][module]
    ours = views["ours"][module]
    assert ours == theirs


@pytest.mark.parametrize("module", MODULES)
def test_names_defined(module: str) -> None:
    spec = json.dumps({"modules": [module]})
    theirs = LIST.validate_python(run_probe(LITELLM, "defined", spec)[module])
    ours = LIST.validate_python(run_probe(BETTERLITE, "defined", spec)[module])
    assert ours == [name for name in theirs if name in MODULES[module]]
    assert set(ours) <= set(MODULES[module])


def test_names_behavior() -> None:
    theirs = LIST.validate_python(run_probe(LITELLM, "behavior", BEHAVIOR_SPEC)["cases"])
    ours = LIST.validate_python(run_probe(BETTERLITE, "behavior", BEHAVIOR_SPEC)["cases"])
    assert ours == theirs
    assert len(theirs) == len(CASES)
    dumped = json.dumps(theirs, sort_keys=True)
    assert dumped.count('"error"') > REFUSAL
    assert dumped.count('"dump"') > REFUSAL


def test_names_special_agents() -> None:
    theirs = run_probe(LITELLM, "special", AGENT_SPECIAL_SPEC)
    ours = run_probe(BETTERLITE, "special", AGENT_SPECIAL_SPEC)
    assert ours == theirs
    assert ours["headers"] == ["x-litellm-user-id", "x-litellm-team-id"]
    assert ours["succeeded"] == [False, False, True, True, False]


def test_names_special_decisions() -> None:
    theirs = run_probe(LITELLM, "special", DECISION_SPECIAL_SPEC)
    ours = run_probe(BETTERLITE, "special", DECISION_SPECIAL_SPEC)
    assert ours == theirs
    assert ours["choice_keys"] == ["true", "false", "true"]
    assert ours["frozen"] == ["rejected"]
    assert ours["usage"] == {"input_tokens": 0, "output_tokens": 0}
