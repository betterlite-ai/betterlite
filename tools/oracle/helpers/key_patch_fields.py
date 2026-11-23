import asyncio
import logging
from collections.abc import Mapping
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy._types import KeyRequestBase, ProxyException, UpdateKeyRequest
from litellm.proxy.management_endpoints.key_management_endpoints import (
    _apply_soft_budget_update,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2407-L2423
    _requested_end_user_budget_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2985-L2991
    _require_prisma_client,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2979-L2982
    _resolve_token_to_update,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2685-L2695
    _validate_max_budget,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2597-L2611
    _validate_soft_budget_value,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2376-L2381
    _verification_token_from_row,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L361-L363
    is_different_team,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2589-L2594
    prepare_key_update_data,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2460-L2563
    prepare_metadata_fields,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2325-L2373
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)
_MAP: Final = TypeAdapter(dict[str, JsonValue])
_PROXY_LOGGER: Final = "LiteLLM Proxy"
_WARNING_SUBJECT: Final = "metadata"


class MetadataCall(TypedDict):
    function: Literal["metadata"]
    request: dict[str, JsonValue]
    existing_metadata: dict[str, JsonValue]


class BudgetCheckCall(TypedDict):
    function: Literal["budget_check"]
    field: Literal["soft", "max"]
    value: float | None


class SoftBudgetCall(TypedDict):
    function: Literal["soft_budget_update"]
    request: dict[str, JsonValue]
    existing: dict[str, JsonValue]
    non_default_values: dict[str, JsonValue]


class TokenCall(TypedDict):
    function: Literal["token"]
    request_key: str | None
    existing_token: str | None


class TeamCall(TypedDict):
    function: Literal["team"]
    request_team: str | None
    existing_team: str | None


class EndUserCall(TypedDict):
    function: Literal["end_user"]
    request: dict[str, JsonValue]


class DatabaseCall(TypedDict):
    function: Literal["database"]


class PrepareCall(TypedDict):
    function: Literal["prepare"]
    request: dict[str, JsonValue]
    existing: dict[str, JsonValue]


Call: TypeAlias = (
    MetadataCall
    | BudgetCheckCall
    | SoftBudgetCall
    | TokenCall
    | TeamCall
    | EndUserCall
    | DatabaseCall
    | PrepareCall
)


class Input(TypedDict):
    calls: list[Call]


class Envelope(TypedDict):
    value: JsonValue
    error: JsonValue
    warnings: list[JsonValue]
    effects: list[JsonValue]


# the source logs the aborted projection loop and keeps the request, so a record is the warning
class _WarningHandler(logging.Handler):
    subjects: list[JsonValue]

    def __init__(self) -> None:
        super().__init__(logging.ERROR)
        self.subjects = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.ERROR:
            self.subjects.append(_WARNING_SUBJECT)


class _CreatedBudget:
    budget_id: str = "budget-created"


class _Budgets:
    effects: list[JsonValue]

    def __init__(self, effects: list[JsonValue]) -> None:
        self.effects = effects

    async def update(
        self,
        *,
        where: Mapping[str, JsonValue],
        data: Mapping[str, JsonValue],
    ) -> None:
        effect: JsonValue = {
            "kind": "update",
            "where": {"budget_id": where["budget_id"]},
            "data": {"soft_budget": data["soft_budget"], "updated_by": data["updated_by"]},
        }
        self.effects.append(effect)

    async def create(self, *, data: Mapping[str, JsonValue]) -> _CreatedBudget:
        effect: JsonValue = {
            "kind": "create",
            "data": {
                "soft_budget": data["soft_budget"],
                "created_by": data["created_by"],
                "updated_by": data["updated_by"],
            },
        }
        self.effects.append(effect)
        return _CreatedBudget()


class _FakeTx:
    litellm_budgettable: _Budgets

    def __init__(self, budgets: _Budgets) -> None:
        self.litellm_budgettable = budgets


def _http_failure(error: HTTPException, warnings: list[JsonValue]) -> Envelope:
    encoded: JsonValue = {"status": error.status_code, "type": None, "param": None, "code": None}
    return {"value": None, "error": encoded, "warnings": warnings, "effects": []}


def _proxy_failure(error: ProxyException, warnings: list[JsonValue]) -> Envelope:
    encoded: JsonValue = {
        "status": int(error.code), "type": error.type, "param": error.param, "code": error.code,
    }
    return {"value": None, "error": encoded, "warnings": warnings, "effects": []}


def _metadata_answer(call: MetadataCall, warnings: list[JsonValue]) -> Envelope:
    data = UpdateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    supplied: dict[str, JsonValue] = _MAP.validate_json(data.model_dump_json(exclude_unset=True))
    values: dict[str, JsonValue] = prepare_metadata_fields(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2325-L2373
        data, supplied, dict(call["existing_metadata"])
    )
    projected = _MAP.validate_python(values)
    return {"value": projected, "error": None, "warnings": warnings, "effects": []}


def _budget_answer(call: BudgetCheckCall) -> Envelope:
    if call["field"] == "soft":
        _validate_soft_budget_value(call["value"])
    else:
        _validate_max_budget(call["value"])
    return {"value": None, "error": None, "warnings": [], "effects": []}


def _soft_budget_answer(call: SoftBudgetCall) -> Envelope:
    data = UpdateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row(call["existing"])
    effects: list[JsonValue] = []
    values = asyncio.run(
        _apply_soft_budget_update(
            data=data,
            non_default_values=call["non_default_values"],
            db=_FakeTx(_Budgets(effects)),  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2407-L2423
            existing_key_row=existing,
            changed_by="admin",
        )
    )
    projected = _MAP.validate_python(dict(values))
    return {"value": projected, "error": None, "warnings": [], "effects": effects}


def _token_answer(call: TokenCall) -> Envelope:
    # the model needs an identifier that _resolve_token_to_update never reads
    body: dict[str, JsonValue] = {"key": call["request_key"], "key_alias": "fallback"}
    data = UpdateKeyRequest.model_validate(body)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row({"token": call["existing_token"]})
    token = _resolve_token_to_update(data, existing)
    return {"value": token, "error": None, "warnings": [], "effects": []}


def _team_answer(call: TeamCall) -> Envelope:
    body: dict[str, JsonValue] = {"key": "probe", "team_id": call["request_team"]}
    data = UpdateKeyRequest.model_validate(body)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row({"team_id": call["existing_team"]})
    changed = is_different_team(data, existing)
    return {"value": changed, "error": None, "warnings": [], "effects": []}


def _end_user_answer(call: EndUserCall) -> Envelope:
    data = KeyRequestBase.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1340
    budget_id = _requested_end_user_budget_id(data)
    return {"value": budget_id, "error": None, "warnings": [], "effects": []}


def _database_answer() -> Envelope:
    _require_prisma_client(None)
    return {"value": None, "error": None, "warnings": [], "effects": []}


def _prepare_answer(call: PrepareCall, warnings: list[JsonValue]) -> Envelope:
    data = UpdateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row(call["existing"])
    values: Mapping[str, JsonValue] = asyncio.run(prepare_key_update_data(data, existing))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2460-L2563
    projected = _MAP.validate_python(dict(values))
    return {"value": projected, "error": None, "warnings": warnings, "effects": []}


def answer(call: Call, warnings: list[JsonValue]) -> Envelope:
    envelope: Envelope = {"value": None, "error": None, "warnings": warnings, "effects": []}
    match call["function"]:
        case "metadata":
            envelope = _metadata_answer(call, warnings)
        case "budget_check":
            envelope = _budget_answer(call)
        case "soft_budget_update":
            envelope = _soft_budget_answer(call)
        case "token":
            envelope = _token_answer(call)
        case "team":
            envelope = _team_answer(call)
        case "end_user":
            envelope = _end_user_answer(call)
        case "database":
            envelope = _database_answer()
        case "prepare":
            envelope = _prepare_answer(call, warnings)
    return envelope


def call_outcome(call: Call) -> JsonValue:
    handler = _WarningHandler()
    logger = logging.getLogger(_PROXY_LOGGER)
    logger.addHandler(handler)
    try:
        envelope = answer(call, handler.subjects)
    except HTTPException as error:
        envelope = _http_failure(error, handler.subjects)
    except ProxyException as error:
        envelope = _proxy_failure(error, handler.subjects)
    finally:
        logger.removeHandler(handler)
    return _JSON.validate_python(envelope)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    outcomes: list[JsonValue] = [call_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
