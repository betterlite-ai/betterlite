import asyncio
import logging
from collections.abc import Mapping
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy._types import ProxyException, UpdateKeyRequest
from litellm.proxy.management_endpoints.key_management_endpoints import (
    _apply_soft_budget_update,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2407-L2423
    _resolve_token_to_update,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2685-L2695
    _verification_token_from_row,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L361-L363
    prepare_key_update_data,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2460-L2563
    prepare_metadata_fields,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2325-L2373
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


class SoftBudgetCall(TypedDict):
    function: Literal["soft_budget_update"]
    request: dict[str, JsonValue]
    existing: dict[str, JsonValue]
    non_default_values: dict[str, JsonValue]


class TokenCall(TypedDict):
    function: Literal["token"]
    request_key: str | None
    existing_token: str | None


class PrepareCall(TypedDict):
    function: Literal["prepare"]
    request: dict[str, JsonValue]
    existing: dict[str, JsonValue]


Call: TypeAlias = MetadataCall | SoftBudgetCall | TokenCall | PrepareCall


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


def _effect(
    kind: str,
    budget_id: JsonValue,
    soft_budget: JsonValue,
    actor: JsonValue,
) -> JsonValue:
    effect: JsonValue = {
        "kind": kind,
        "budget_id": budget_id,
        "soft_budget": soft_budget,
        "actor": actor,
    }
    return effect


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
        effect = _effect("update", where["budget_id"], data["soft_budget"], data["updated_by"])
        self.effects.append(effect)

    async def create(self, *, data: Mapping[str, JsonValue]) -> _CreatedBudget:
        effect = _effect(
            "create", _CreatedBudget.budget_id, data["soft_budget"], data["created_by"]
        )
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
    projected: dict[str, JsonValue] = _MAP.validate_python(prepare_metadata_fields(data, supplied, dict(call["existing_metadata"])))  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2325-L2373
    return {"value": projected, "error": None, "warnings": warnings, "effects": []}


def _soft_budget_answer(call: SoftBudgetCall) -> Envelope:
    data = UpdateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row(call["existing"])
    effects: list[JsonValue] = []
    budgets = _Budgets(effects)
    projected: dict[str, JsonValue] = _MAP.validate_python(asyncio.run(_apply_soft_budget_update(data=data, non_default_values=call["non_default_values"], db=_FakeTx(budgets), existing_key_row=existing, changed_by="admin")))  # type: ignore[arg-type]  # noqa: E501  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2407-L2423
    return {"value": projected, "error": None, "warnings": [], "effects": effects}


def _token_answer(call: TokenCall) -> Envelope:
    # the model needs an identifier that _resolve_token_to_update never reads
    body: dict[str, JsonValue] = {"key": call["request_key"], "key_alias": "fallback"}
    data = UpdateKeyRequest.model_validate(body)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row({"token": call["existing_token"]})
    token = _resolve_token_to_update(data, existing)
    return {"value": token, "error": None, "warnings": [], "effects": []}


def _prepare_answer(call: PrepareCall, warnings: list[JsonValue]) -> Envelope:
    data = UpdateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    existing = _verification_token_from_row(call["existing"])
    projected: dict[str, JsonValue] = _MAP.validate_python(asyncio.run(prepare_key_update_data(data, existing)))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L2460-L2563
    return {"value": projected, "error": None, "warnings": warnings, "effects": []}


def answer(call: Call, warnings: list[JsonValue]) -> Envelope:
    envelope: Envelope = {"value": None, "error": None, "warnings": warnings, "effects": []}
    match call["function"]:
        case "metadata":
            envelope = _metadata_answer(call, warnings)
        case "soft_budget_update":
            envelope = _soft_budget_answer(call)
        case "token":
            envelope = _token_answer(call)
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
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        envelope = _proxy_failure(error, handler.subjects)
    finally:
        logger.removeHandler(handler)
    return _JSON.validate_python(envelope)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    outcomes: list[JsonValue] = [call_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
