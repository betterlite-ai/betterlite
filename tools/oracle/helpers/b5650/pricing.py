from importlib import import_module
from typing import Final, Literal, Protocol, TypedDict, cast

from litellm.types import utils as pricing_types

from b5365.registration import Entry, Json, project
from b5650 import fields
from b5650.fields import UTILS


class Main(Protocol):
    def _build_custom_pricing_entry(
        self, custom_llm_provider: str, kwargs: Entry, model_info: Entry | None = None
    ) -> Entry: ...

    def _get_router_deployment_id(self, kwargs: Entry) -> str | None: ...


MAIN: Final[Main] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1244-L1281
        Main, import_module("litellm.main")
    )
)


class PricingEntry(TypedDict):
    function: Literal["pricing_entry"]
    custom_llm_provider: str
    kwargs: Entry
    model_info: Entry | None


class DeploymentId(TypedDict):
    function: Literal["deployment_id"]
    kwargs: Entry


class CustomFields(TypedDict):
    function: Literal["custom_fields"]


class SharedFields(TypedDict):
    function: Literal["shared_fields"]


class Constants(TypedDict):
    function: Literal["constants"]


PricingInput = (
    PricingEntry | DeploymentId | CustomFields | SharedFields | Constants | fields.FieldsInput
)


def run(call: PricingInput) -> Json:
    if call["function"] == "pricing_entry":
        entry = MAIN._build_custom_pricing_entry(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1244
            custom_llm_provider=call["custom_llm_provider"],
            kwargs=call["kwargs"],
            model_info=call["model_info"],
        )
        return project(entry)
    if call["function"] == "deployment_id":
        return MAIN._get_router_deployment_id(call["kwargs"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1270
    if call["function"] == "custom_fields":
        return sorted(UTILS.CustomPricingLiteLLMParams.model_fields)
    if call["function"] == "shared_fields":
        return sorted(pricing_types.SHARED_BACKEND_MODEL_INFO_FIELDS)
    if call["function"] == "constants":
        return {
            "deployment_scoped": sorted(pricing_types.DEPLOYMENT_SCOPED_PRICING_FIELDS),
            "exemptions": sorted(
                frozenset(UTILS.CustomPricingLiteLLMParams.model_fields)
                - pricing_types.SERVER_DERIVED_PRICING_FIELDS
            ),
            "overrides_key": pricing_types.PRICING_OVERRIDES_KEY,
            "lookup_key": pricing_types.COST_MAP_LOOKUP_KEY,
        }
    return fields.run(call)
