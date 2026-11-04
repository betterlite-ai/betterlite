from collections.abc import Mapping
from typing import Protocol, TypeAlias, cast

from litellm.types.vector_stores import (
    IndexCreateRequest,
    IndexListResponse,
    LiteLLM_ManagedVectorStoreIndex,
    VectorStoreDeleteRequest,
    VectorStoreInfoRequest,
    VectorStoreToolParams,
    VectorStoreUpdateRequest,
)

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
    if operation == "management_record_defaults":
        records = cast(dict[str, Json], fields["records"])
        validated: dict[str, Json] = {}
        models = {
            "update": VectorStoreUpdateRequest,
            "delete": VectorStoreDeleteRequest,
            "info": VectorStoreInfoRequest,
            "managed_index": LiteLLM_ManagedVectorStoreIndex,
            "index_list": IndexListResponse,
            "index_create": IndexCreateRequest,
        }
        for name, model in models.items():
            instance = model.model_validate(records[name])
            validated[name] = cast(Json, instance.model_dump(mode="json", exclude_none=False))
        return cast(Json, validated)
    if operation == "managed_index_timestamp_forms":
        records = cast(list[dict[str, Json]], fields["records"])
        validated_records = [
            LiteLLM_ManagedVectorStoreIndex.model_validate(record) for record in records
        ]
        return cast(
            Json,
            [record.model_dump(mode="json") for record in validated_records],
        )
    if operation in {"invalid_index_list_literal", "invalid_missing_id", "invalid_index_info"}:
        models = {
            "invalid_index_list_literal": IndexListResponse,
            "invalid_missing_id": VectorStoreDeleteRequest,
            "invalid_index_info": IndexCreateRequest,
        }
        model = models[operation]
        instance = model.model_validate(fields["record"])
        return cast(Json, instance.model_dump(mode="json"))
    record_name = "extra_body" if operation == "discovery_extra_body_fields" else "record"
    return fields[record_name]
