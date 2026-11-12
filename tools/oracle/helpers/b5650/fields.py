from collections.abc import Mapping
from importlib import import_module
from typing import Final, Literal, Protocol, TypedDict, cast

from pydantic.fields import FieldInfo

from b5365.registration import Entry, Json, project


class PricingParams(Protocol):
    model_fields: Mapping[str, FieldInfo]

    def strip_custom_pricing_fields(self, model_info: Entry) -> Entry: ...


class Utils(Protocol):
    CustomPricingLiteLLMParams: PricingParams

    def shared_backend_model_info(self, model_info: Entry) -> Entry: ...

    def is_server_derived_pricing_key(self, key: str) -> bool: ...

    def without_server_derived_pricing(self, model_info: Entry) -> Mapping[str, Json]: ...

    def echoed_cost_map_pricing_fields(self, model_info: Entry) -> tuple[str, ...]: ...

    def echoed_cost_map_fields(
        self, model_info: Entry, *cost_map_entries: Entry
    ) -> tuple[str, ...]: ...

    def pricing_override_fields(self, *sources: Entry) -> tuple[str, ...]: ...


UTILS: Final[Utils] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3727-L3953
        Utils, import_module("litellm.types.utils")
    )
)


class ModelInfoOp(TypedDict):
    function: Literal["strip", "shared_info", "without_server_derived", "echoed_pricing"]
    model_info: Entry


class ServerDerivedKey(TypedDict):
    function: Literal["server_derived_key"]
    key: str


class EchoedFields(TypedDict):
    function: Literal["echoed_fields"]
    model_info: Entry
    entries: list[Entry]


class OverrideFields(TypedDict):
    function: Literal["override_fields"]
    sources: list[Entry]


FieldsInput = ModelInfoOp | ServerDerivedKey | EchoedFields | OverrideFields


def run(call: FieldsInput) -> Json:
    if call["function"] == "server_derived_key":
        return UTILS.is_server_derived_pricing_key(call["key"])
    if call["function"] == "echoed_fields":
        return list(UTILS.echoed_cost_map_fields(call["model_info"], *call["entries"]))
    if call["function"] == "override_fields":
        return list(UTILS.pricing_override_fields(*call["sources"]))
    if call["function"] == "echoed_pricing":
        return list(UTILS.echoed_cost_map_pricing_fields(call["model_info"]))
    entry: Mapping[str, Json]
    if call["function"] == "strip":
        entry = UTILS.CustomPricingLiteLLMParams.strip_custom_pricing_fields(call["model_info"])
    elif call["function"] == "shared_info":
        entry = UTILS.shared_backend_model_info(call["model_info"])
    else:
        entry = UTILS.without_server_derived_pricing(call["model_info"])
    return project(entry)
