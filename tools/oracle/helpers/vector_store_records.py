from collections.abc import Mapping
from typing import Protocol, TypeAlias, cast

from litellm.types.vector_stores import VectorStoreToolParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class SupportsToDict(Protocol):
    def to_dict(self) -> Mapping[str, Json]: ...


def run(fields: dict[str, Json]) -> Json:
    operation = fields["operation"]
    if operation == "tool_params_omit_only_null":
        params = VectorStoreToolParams(
            filters=cast(dict[str, object] | None, fields.get("filters")),
            max_num_results=cast(int | None, fields.get("max_num_results")),
            ranking_options=cast(dict[str, object] | None, fields.get("ranking_options")),
        )
        return cast(Json, dict(cast(SupportsToDict, params).to_dict()))
    return cast(Json, {key: value for key, value in fields.items() if key != "operation"})
