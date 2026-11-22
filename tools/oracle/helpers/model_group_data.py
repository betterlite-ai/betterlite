from dataclasses import dataclass
from datetime import datetime
from typing import Final, Literal, TypedDict

import litellm
from fastapi import HTTPException
from litellm.proxy.management_endpoints import (
    model_access_group_management_endpoints as group_endpoints,
)
from litellm.proxy.management_endpoints.model_access_group_management_endpoints import (
    add_access_group_to_deployment,
    remove_access_group_from_deployment,
    validate_models_exist,
)
from litellm.router import Router
from litellm.types.proxy.management_endpoints.model_management_endpoints import (
    AccessGroupInfo,
)
from litellm.types.proxy.model_access_group_budget import ModelAccessGroupBudget
from pydantic import JsonValue, TypeAdapter


class Tag(TypedDict):
    function: str


class FlattenedBudgetCall(TypedDict):
    function: Literal["flattened_budget"]
    record: dict[str, JsonValue]


class WithBudgetCall(TypedDict):
    function: Literal["with_budget"]
    info: dict[str, JsonValue]
    row: dict[str, JsonValue] | None


class BudgetOrNoneCall(TypedDict):
    function: Literal["budget_or_none"]
    row: dict[str, JsonValue] | None


class BudgetResponseCall(TypedDict):
    function: Literal["budget_response"]
    access_group: str
    row: dict[str, JsonValue] | None


class ValidateCall(TypedDict):
    function: Literal["validate_models_exist"]
    model_names: list[str]
    router_names: list[str] | None


class GroupCall(TypedDict):
    function: Literal["add_access_group_to_deployment", "remove_access_group_from_deployment"]
    model_info: dict[str, JsonValue]
    access_group: str


class ReadableCall(TypedDict):
    function: Literal["readable_model_info_or_raise"]
    model_id: str
    model_info: JsonValue


class WrittenInput(TypedDict):
    id: str
    model_info: dict[str, JsonValue]


class RaiseCall(TypedDict):
    function: Literal["raise_http_if_reload_degraded_serving"]
    before: list[str]
    written: list[WrittenInput]
    access_group: str
    still_desired: list[str] | None
    live_after: list[str]


class Input(TypedDict):
    calls: list[JsonValue]


TAG: Final = TypeAdapter(Tag)
FLATTENED_BUDGET: Final = TypeAdapter(FlattenedBudgetCall)
WITH_BUDGET: Final = TypeAdapter(WithBudgetCall)
BUDGET_OR_NONE: Final = TypeAdapter(BudgetOrNoneCall)
BUDGET_RESPONSE: Final = TypeAdapter(BudgetResponseCall)
VALIDATE: Final = TypeAdapter(ValidateCall)
GROUP: Final = TypeAdapter(GroupCall)
READABLE: Final = TypeAdapter(ReadableCall)
RAISE: Final = TypeAdapter(RaiseCall)
INFO: Final = TypeAdapter(AccessGroupInfo)
RECORD: Final = TypeAdapter(ModelAccessGroupBudget)
JSON: Final = TypeAdapter(JsonValue)


@dataclass
class BudgetRow:
    budget_id: str
    max_budget: float | None
    soft_budget: float | None
    budget_duration: str | None
    budget_reset_at: datetime | None


@dataclass
class GroupBudgetRow:
    access_group_name: str
    spend: float
    budget_id: str | None
    litellm_budget_table: BudgetRow | None


ROW: Final = TypeAdapter(GroupBudgetRow)


def row_of(value: dict[str, JsonValue] | None) -> GroupBudgetRow | None:
    if value is None:
        return None
    return ROW.validate_python(value)


def flattened_budget_answer(call: FlattenedBudgetCall) -> JsonValue:
    record = RECORD.validate_python(call["record"])
    return {"ok": True, "value": JSON.validate_json(record.model_dump_json())}


def with_budget_answer(call: WithBudgetCall) -> JsonValue:
    info = INFO.validate_python(call["info"])
    updated = group_endpoints._with_budget(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_access_group_management_endpoints.py#L226
        info, row_of(call["row"])
    )
    return {"ok": True, "value": JSON.validate_json(updated.model_dump_json())}


def budget_or_none_answer(call: BudgetOrNoneCall) -> JsonValue:
    budget = group_endpoints._budget_or_none(row_of(call["row"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_access_group_management_endpoints.py#L238
    if budget is None:
        return {"ok": True, "value": None}
    return {"ok": True, "value": JSON.validate_json(budget.model_dump_json())}


def budget_response_answer(call: BudgetResponseCall) -> JsonValue:
    response = group_endpoints._budget_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_access_group_management_endpoints.py#L251
        call["access_group"], row_of(call["row"])
    )
    return {"ok": True, "value": JSON.validate_json(response.model_dump_json())}


def validate_answer(call: ValidateCall) -> JsonValue:
    router: Router | None = None
    names = call["router_names"]
    if names is not None:
        model_list: list[dict[str, JsonValue]] = []
        for name in names:
            params: dict[str, JsonValue] = {"model": f"openai/{name}"}
            model_list.append({"model_name": name, "litellm_params": params})
        router = Router(model_list=model_list)
    all_valid, missing = validate_models_exist(call["model_names"], router)
    report = {"all_valid": all_valid, "missing": list(missing)}
    return {"ok": True, "value": JSON.validate_python(report)}


def group_answer(call: GroupCall) -> JsonValue:
    if call["function"] == "add_access_group_to_deployment":
        updated, modified = add_access_group_to_deployment(
            call["model_info"], call["access_group"]
        )
    else:
        updated, modified = remove_access_group_from_deployment(
            call["model_info"], call["access_group"]
        )
    report = {"model_info": JSON.validate_python(updated), "modified": modified}
    return {"ok": True, "value": JSON.validate_python(report)}


def readable_answer(call: ReadableCall) -> JsonValue:
    try:
        mapping = group_endpoints._readable_model_info_or_raise(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_access_group_management_endpoints.py#L396
            model_id=call["model_id"], model_info=call["model_info"]
        )
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "value": JSON.validate_python(dict(mapping))}


def frozen(ids: list[str] | None) -> frozenset[str] | None:
    if ids is None:
        return None
    return frozenset(ids)


def raise_answer(call: RaiseCall) -> JsonValue:
    written = [(model["id"], model["model_info"]) for model in call["written"]]
    try:
        group_endpoints._raise_http_if_reload_degraded_serving(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_access_group_management_endpoints.py#L339
            before=frozenset(call["before"]),
            written_models=written,
            access_group=call["access_group"],
            still_desired=frozen(call["still_desired"]),
            live_after=frozenset(call["live_after"]),
        )
    except HTTPException as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "value": None}


def budget_answer(call: JsonValue, function: str) -> JsonValue:
    if function == "flattened_budget":
        return flattened_budget_answer(FLATTENED_BUDGET.validate_python(call))
    if function == "with_budget":
        return with_budget_answer(WITH_BUDGET.validate_python(call))
    if function == "budget_or_none":
        return budget_or_none_answer(BUDGET_OR_NONE.validate_python(call))
    if function == "budget_response":
        return budget_response_answer(BUDGET_RESPONSE.validate_python(call))
    raise ValueError(f"unknown budget function {function}")


def answer(call: JsonValue) -> JsonValue:
    function = TAG.validate_python(call)["function"]
    if function in ("flattened_budget", "with_budget", "budget_or_none", "budget_response"):
        return budget_answer(call, function)
    if function == "validate_models_exist":
        return validate_answer(VALIDATE.validate_python(call))
    if function in ("add_access_group_to_deployment", "remove_access_group_from_deployment"):
        return group_answer(GROUP.validate_python(call))
    if function == "readable_model_info_or_raise":
        return readable_answer(READABLE.validate_python(call))
    if function == "raise_http_if_reload_degraded_serving":
        return raise_answer(RAISE.validate_python(call))
    raise ValueError(f"unknown function {function}")


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    answers = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
