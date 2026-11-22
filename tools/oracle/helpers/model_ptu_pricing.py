from typing import Final, Literal, TypedDict

import litellm
from fastapi import HTTPException
from litellm.proxy.management_endpoints import model_management_endpoints
from litellm.types.router import Deployment, LiteLLM_Params, ModelInfo, updateDeployment
from litellm.types.utils import echoed_cost_map_fields
from pydantic import JsonValue, TypeAdapter
from pydantic_core import to_json


class Tag(TypedDict):
    function: str


class PricesCall(TypedDict):
    function: Literal["prices"]
    values: list[JsonValue]


class PricedCall(TypedDict):
    function: Literal["priced"]
    model_info: dict[str, JsonValue]
    supplied: dict[str, JsonValue]


class ZeroedCall(TypedDict):
    function: Literal["zeroed"]
    model_info: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    supplied: dict[str, JsonValue]


class DeltaCall(TypedDict):
    function: Literal["delta"]
    stored_model_info: dict[str, JsonValue]
    model_info: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    patch: updateDeployment


class DeploymentCall(TypedDict):
    function: Literal["priced_deployment"]
    litellm_params: LiteLLM_Params
    model_info: ModelInfo


class CatalogCall(TypedDict):
    function: Literal["catalog"]
    cost_map: dict[str, dict[str, JsonValue]]
    catalog: dict[str, dict[str, JsonValue]]
    model: str
    incoming: dict[str, JsonValue]


class Input(TypedDict):
    salt_key: str
    ptu_enabled: bool
    calls: list[JsonValue]


JSON: Final = TypeAdapter(JsonValue)
JSON_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
TAG: Final = TypeAdapter(Tag)
PRICES_CALL: Final = TypeAdapter(PricesCall)
PRICED_CALL: Final = TypeAdapter(PricedCall)
ZEROED_CALL: Final = TypeAdapter(ZeroedCall)
DELTA_CALL: Final = TypeAdapter(DeltaCall)
DEPLOYMENT_CALL: Final = TypeAdapter(DeploymentCall)
CATALOG_CALL: Final = TypeAdapter(CatalogCall)


class LoadedCatalog:
    entries: dict[str, dict[str, JsonValue]]
    loads: int

    def __init__(self, entries: dict[str, dict[str, JsonValue]]) -> None:
        self.entries = entries
        self.loads = 0

    def load(self) -> dict[str, dict[str, JsonValue]]:
        self.loads += 1
        return self.entries


def prices_answer(call: PricesCall) -> JsonValue:
    answers: list[JsonValue] = [
        [
            model_management_endpoints._is_nonzero_rate(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L812-L813
            model_management_endpoints._is_nonzero_price(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L816-L819
            model_management_endpoints._is_zero_price(value),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L822-L827
        ]
        for value in call["values"]
    ]
    return {"answers": answers}


def priced_answer(call: PricedCall) -> JsonValue:
    try:
        model_management_endpoints._raise_if_ptu_deployment_is_priced(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L830-L855
            model_info=call["model_info"], supplied=call["supplied"]
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L849-L855
        failed: JsonValue = {"status": error.status_code, "type": None, "param": None, "code": None}
        return {"error": failed}
    return {"error": None}


def zeroed_answer(call: ZeroedCall) -> JsonValue:
    try:
        pricing = model_management_endpoints._ptu_zeroed_pricing(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L858-L895
            model_info=call["model_info"],
            litellm_params=call["litellm_params"],
            supplied=call["supplied"],
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L849-L855
        failed: JsonValue = {"status": error.status_code, "type": None, "param": None, "code": None}
        return {"pricing": None, "error": failed}
    return {"pricing": JSON.validate_json(to_json(dict(pricing))), "error": None}


def delta_answer(call: DeltaCall) -> JsonValue:
    try:
        pricing, released = model_management_endpoints._ptu_pricing_delta(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L898-L928
            stored_model_info=call["stored_model_info"],
            model_info=call["model_info"],
            litellm_params=call["litellm_params"],
            patch=call["patch"],
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L849-L855
        failed: JsonValue = {"status": error.status_code, "type": None, "param": None, "code": None}
        return {"written": None, "released": None, "error": failed}
    dropped: list[JsonValue] = []
    dropped.extend(sorted(released))
    return {
        "written": JSON.validate_json(to_json(dict(pricing))),
        "released": dropped,
        "error": None,
    }


def deployment_answer(call: DeploymentCall) -> JsonValue:
    deployment = Deployment(
        model_name="ptu", litellm_params=call["litellm_params"], model_info=call["model_info"]
    )
    try:
        priced = model_management_endpoints._ptu_priced_deployment(deployment)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L931-L950
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L849-L855
        failed: JsonValue = {"status": error.status_code, "type": None, "param": None, "code": None}
        return {"litellm_params": None, "model_info": None, "error": failed}
    litellm_params = priced.litellm_params.model_dump_json(exclude_none=True)
    model_info = priced.model_info.model_dump_json(exclude_none=True)
    return {
        "litellm_params": JSON_OBJECT.validate_json(litellm_params),
        "model_info": JSON_OBJECT.validate_json(model_info),
        "error": None,
    }


def catalog_answer(call: CatalogCall) -> JsonValue:
    stored = Deployment(
        model_name="ptu", litellm_params=LiteLLM_Params(model=call["model"]), model_info=ModelInfo()
    )
    catalog = LoadedCatalog(call["catalog"])
    runtime_map = litellm.model_cost
    litellm.model_cost = call["cost_map"]
    try:
        cost_entry = model_management_endpoints._cost_map_entry(stored, call["incoming"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L953-L960
    finally:
        litellm.model_cost = runtime_map
    catalog_entry = model_management_endpoints._loaded_catalog_entry(call["incoming"], catalog.load)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L966-L972
    echoed: list[JsonValue] = []
    echoed.extend(echoed_cost_map_fields(call["incoming"], cost_entry, catalog_entry))
    return {
        "echoed": echoed,
        "catalog_entry": JSON.validate_json(to_json(dict(catalog_entry))),
        "catalog_loaded": catalog.loads > 0,
    }


def answer(call: JsonValue) -> JsonValue:
    function = TAG.validate_python(call)["function"]
    if function == "prices":
        return prices_answer(PRICES_CALL.validate_python(call))
    if function == "priced":
        return priced_answer(PRICED_CALL.validate_python(call))
    if function == "zeroed":
        return zeroed_answer(ZEROED_CALL.validate_python(call))
    if function == "delta":
        return delta_answer(DELTA_CALL.validate_python(call))
    if function == "priced_deployment":
        return deployment_answer(DEPLOYMENT_CALL.validate_python(call))
    return catalog_answer(CATALOG_CALL.validate_python(call))


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
