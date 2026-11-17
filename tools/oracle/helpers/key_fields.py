from datetime import datetime
from typing import Final, Literal, TypedDict

from litellm.proxy import utils
from litellm.proxy._types import (
    GenerateKeyRequest,
    GenerateKeyResponse,
    GenerateRequestBase,
    KeyRequestBase,
    RegenerateKeyRequest,
    UpdateKeyRequest,
)
from litellm.proxy.management_helpers.key_settings_audit import with_settings_updated_at
from litellm.types.proxy.management_endpoints.key_management_endpoints import (
    BulkUpdateTeamKeysRequest,
    CustomKeyPolicyRequest,
    KeyUpdateFields,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)

MODELS: Final[dict[str, type[BaseModel]]] = {
    "base": GenerateRequestBase,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1289
    "key": KeyRequestBase,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1340
    "generate": GenerateKeyRequest,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1376
    "update": UpdateKeyRequest,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    "regenerate": RegenerateKeyRequest,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1487
    "response": GenerateKeyResponse,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1399
    "broadcast": KeyUpdateFields,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/management_endpoints/key_management_endpoints.py#L72
}


class ProjectionCall(TypedDict):
    function: Literal[
        "base",
        "key",
        "generate",
        "update",
        "regenerate",
        "response",
        "broadcast",
    ]
    body: JsonValue


class TeamKeysCall(TypedDict):
    function: Literal["team_keys"]
    body: JsonValue


class PolicyCall(TypedDict):
    function: Literal["policy"]
    body: JsonValue


class SettingsCall(TypedDict):
    function: Literal["settings"]
    body: dict[str, JsonValue]
    now: str


class TokenCall(TypedDict):
    function: Literal["token"]
    token: str


Call = ProjectionCall | TeamKeysCall | PolicyCall | SettingsCall | TokenCall


class Input(TypedDict):
    calls: list[Call]


def dump_projection(model: BaseModel) -> JsonValue:
    supplied = _JSON.validate_json(model.model_dump_json(exclude_unset=True))
    resolved = _JSON.validate_json(model.model_dump_json())
    return {"supplied": supplied, "resolved": resolved}


def projection_answer(call: ProjectionCall) -> JsonValue:
    model = MODELS[call["function"]]
    return dump_projection(model.model_validate(call["body"]))


def team_keys_answer(call: TeamKeysCall) -> JsonValue:
    request = BulkUpdateTeamKeysRequest.model_validate(call["body"])
    return _JSON.validate_json(request.model_dump_json())


def policy_answer(call: PolicyCall) -> JsonValue:
    record = CustomKeyPolicyRequest.model_validate(call["body"])
    request = record.request
    view: dict[str, JsonValue] = {
        "operation": record.operation,
        "request_kind": type(request).__name__,
        "request": _JSON.validate_json(request.model_dump_json()),
    }
    return view


def settings_answer(call: SettingsCall) -> JsonValue:
    stamped = with_settings_updated_at(call["body"])
    stamped["settings_updated_at"] = datetime.fromisoformat(call["now"]).isoformat()
    return _JSON.validate_python(stamped)


def token_answer(call: TokenCall) -> JsonValue:
    return _JSON.validate_python(utils._hash_token_if_needed(call["token"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L7188-L7197


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "base" | "key" | "generate" | "update" | "regenerate" | "response" | "broadcast":
            return projection_answer(call)
        case "team_keys":
            return team_keys_answer(call)
        case "policy":
            return policy_answer(call)
        case "settings":
            return settings_answer(call)
        case "token":
            return token_answer(call)


def call_outcome(call: Call) -> JsonValue:
    try:
        return {"ok": True, "value": answer(call)}
    except (ValueError, TypeError, KeyError):
        return {"ok": False}


def run(input_value: Input) -> JsonValue:
    outcomes = [call_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
