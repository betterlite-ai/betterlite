import asyncio
from typing import Final, Literal, TypedDict

import litellm
from fastapi import HTTPException
from litellm.models.credentials import CredentialItem
from litellm.proxy._types import ProxyException
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints import model_management_endpoints
from litellm.proxy.utils import PrismaClient
from litellm.types.router import (
    Deployment,
    LiteLLM_Params,
    ModelInfo,
    updateDeployment,
    updateLiteLLMParams,
)
from pydantic import JsonValue, TypeAdapter
from pydantic_core import to_json


class Tag(TypedDict):
    function: str


class ParamsCall(TypedDict):
    function: Literal["strategy_router_write", "tuning_write", "effective_params"]
    incoming: updateLiteLLMParams | None
    existing: LiteLLM_Params | None


class MarkerCall(TypedDict):
    function: Literal["member_marker"]
    incoming: updateLiteLLMParams | None
    existing_params: LiteLLM_Params
    existing_model_info: ModelInfo


class CredentialCall(TypedDict):
    function: Literal["credential_name"]
    incoming: updateLiteLLMParams | None
    existing: LiteLLM_Params | None
    loaded: list[str]
    stored: list[str]


class RateLimitCall(TypedDict):
    function: Literal["rate_limits"]
    litellm_params: LiteLLM_Params
    enforced: bool


class PtuCall(TypedDict):
    function: Literal["ptu"]
    stored: ModelInfo
    incoming: ModelInfo | None


class CredentialRow(TypedDict):
    credential_name: str
    credential_values: dict[str, str]
    credential_info: dict[str, str]


class Input(TypedDict):
    salt_key: str
    ptu_enabled: bool
    calls: list[JsonValue]


JSON: Final = TypeAdapter(JsonValue)
JSON_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
TAG: Final = TypeAdapter(Tag)
PARAMS_CALL: Final = TypeAdapter(ParamsCall)
MARKER_CALL: Final = TypeAdapter(MarkerCall)
CREDENTIAL_CALL: Final = TypeAdapter(CredentialCall)
RATE_LIMIT_CALL: Final = TypeAdapter(RateLimitCall)
PTU_CALL: Final = TypeAdapter(PtuCall)


class FakeCredentialTable:
    names: list[str]
    lookups: list[str]

    def __init__(self, names: list[str]) -> None:
        self.names = names
        self.lookups = []

    async def find_unique(self, *, where: dict[str, str]) -> CredentialItem | None:
        name = where["credential_name"]
        self.lookups.append(name)
        if name not in self.names:
            return None
        return CredentialItem.model_validate(
            CredentialRow(credential_name=name, credential_values={}, credential_info={})
        )


class FakeDatabase(PrismaWrapper):
    def __init__(self, credentials: FakeCredentialTable) -> None:
        self.litellm_credentialstable = credentials


class FakePrismaClient(PrismaClient):
    def __init__(self, database: FakeDatabase) -> None:
        self.db = database


def params_answer(call: ParamsCall) -> JsonValue:
    if call["function"] == "effective_params":
        effective = model_management_endpoints._effective_complexity_router_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L459-L468
            call["incoming"], call["existing"]
        )
        params: dict[str, object] = dict(effective)
        return JSON.validate_json(to_json(params))
    try:
        model_management_endpoints._raise_on_strategy_router_write_violation(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L329-L341
            call["incoming"], call["existing"]
        )
        if call["function"] == "tuning_write":
            effective = model_management_endpoints._effective_complexity_router_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L459-L468
                call["incoming"], call["existing"]
            )
            model_management_endpoints._raise_on_tuning_quota_violation(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L508-L517
                candidate=model_management_endpoints._tuning_candidate(effective, "router-1"),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L499-L505
                others=(),
                baselines={},
                limit=None,
            )
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {
            "error": {
                "status": int(error.code),
                "type": error.type,
                "param": error.param,
                "code": error.code,
            }
        }
    return {"error": None}


def marker_answer(call: MarkerCall) -> JsonValue:
    existing = Deployment(
        model_name="router",
        litellm_params=call["existing_params"],
        model_info=call["existing_model_info"],
    )
    marker = model_management_endpoints._member_auto_router_marker_for_update(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L471-L487
        incoming_params=call["incoming"], existing=existing, member_write=None
    )
    return {"marker": marker}


def credential_answer(call: CredentialCall) -> JsonValue:
    litellm.credential_list = [
        CredentialItem.model_validate(
            CredentialRow(credential_name=name, credential_values={}, credential_info={})
        )
        for name in call["loaded"]
    ]
    table = FakeCredentialTable(call["stored"])
    refusal: JsonValue = None
    try:
        asyncio.run(
            model_management_endpoints._raise_on_invalid_credential_name(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L355-L386
                call["incoming"], call["existing"], FakePrismaClient(FakeDatabase(table))
            )
        )
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        refusal = {
            "status": int(error.code),
            "type": error.type,
            "param": error.param,
            "code": error.code,
        }
    lookups: list[JsonValue] = []
    lookups.extend(table.lookups)
    return {"error": refusal, "lookups": lookups}


def rate_limit_answer(call: RateLimitCall) -> JsonValue:
    try:
        model_management_endpoints._raise_if_rate_limits_required_but_missing(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L681-L706
            litellm_params=call["litellm_params"], enforced=call["enforced"]
        )
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {
            "error": {
                "status": int(error.code),
                "type": error.type,
                "param": error.param,
                "code": error.code,
            }
        }
    return {"error": None}


def ptu_answer(call: PtuCall) -> JsonValue:
    incoming = call["incoming"]
    stored = Deployment(
        model_name="router",
        litellm_params=LiteLLM_Params(model="gpt-4o"),
        model_info=call["stored"],
    )
    cleared: list[JsonValue] = []
    cleared.extend(
        sorted(model_management_endpoints._explicitly_cleared_ptu_fields(incoming))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L712-L724
    )
    merged = model_management_endpoints._merged_ptu_model_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L727-L738
        db_model=stored, patch_data=updateDeployment(model_info=incoming)
    )
    supplied: dict[str, JsonValue] = {}
    if incoming is not None:
        supplied = JSON_OBJECT.validate_json(incoming.model_dump_json(exclude_none=True))
    allowed: JsonValue = None
    try:
        model_management_endpoints._raise_if_ptu_cost_attribution_disabled(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L741-L764
            supplied
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L758-L764
        allowed = {"status": error.status_code, "type": None, "param": None, "code": None}
    valid: JsonValue = None
    try:
        model_management_endpoints._validate_ptu_model_info(merged)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L767-L780
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L778-L780
        valid = {"status": error.status_code, "type": None, "param": None, "code": None}
    merged_info: dict[str, object] = dict(merged)
    return {
        "cleared": cleared,
        "merged": JSON.validate_json(to_json(merged_info)),
        "allowed": allowed,
        "valid": valid,
    }


def answer(call: JsonValue) -> JsonValue:
    function = TAG.validate_python(call)["function"]
    if function == "member_marker":
        return marker_answer(MARKER_CALL.validate_python(call))
    if function == "credential_name":
        return credential_answer(CREDENTIAL_CALL.validate_python(call))
    if function == "rate_limits":
        return rate_limit_answer(RATE_LIMIT_CALL.validate_python(call))
    if function == "ptu":
        return ptu_answer(PTU_CALL.validate_python(call))
    return params_answer(PARAMS_CALL.validate_python(call))


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
