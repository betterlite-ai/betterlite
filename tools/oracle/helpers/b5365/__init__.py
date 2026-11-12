from typing import Final, Literal, Protocol, TypedDict, cast

import litellm

from b5365.registration import Entry, Json, Scenario, project, register
from b5365.routing import Deployment, Params, order_filter, region


class Merging(Protocol):
    def _update_dictionary(self, existing_dict: Entry, new_dict: Entry) -> Entry: ...

    def _convert_stringified_numbers(self, value: str) -> Json: ...


UTILS: Final[Merging] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3136-L3320
        Merging, litellm.utils
    )
)


class UpdateDictionary(TypedDict):
    function: Literal["update_dictionary"]
    existing: Entry
    new: Entry


class ConvertNumber(TypedDict):
    function: Literal["convert_stringified_numbers"]
    value: str


class RegisterModel(Scenario):
    function: Literal["register_model"]


class OrderFilter(TypedDict):
    function: Literal["order_filter"]
    deployments: list[Deployment]
    target_order: int | None


class Region(TypedDict):
    function: Literal["region"]
    params: Params
    lists: Json
    default_vertex_location: str | None
    provider: str | None
    allowed_model_region: str


Input = UpdateDictionary | ConvertNumber | RegisterModel | OrderFilter | Region


def run(input_value: Input) -> Json:
    if input_value["function"] == "update_dictionary":
        merged = UTILS._update_dictionary(input_value["existing"], input_value["new"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3136
        return project(merged)
    if input_value["function"] == "convert_stringified_numbers":
        converted = UTILS._convert_stringified_numbers(input_value["value"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3154
        return project(converted)
    if input_value["function"] == "register_model":
        return register(input_value)
    if input_value["function"] == "order_filter":
        return order_filter(input_value["deployments"], input_value["target_order"])
    return region(
        input_value["params"],
        input_value["default_vertex_location"],
        input_value["provider"],
        input_value["allowed_model_region"],
    )
