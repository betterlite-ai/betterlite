from typing import Literal, Protocol, TypedDict, cast

from litellm.types.proxy.management_endpoints.scim_v2 import (
    SCIMGroup,
    SCIMPatchOp,
    SCIMResourceType,
    SCIMSchema,
    SCIMUser,
)


class UserInput(TypedDict):
    kind: Literal["user"]
    value: object


class GroupInput(TypedDict):
    kind: Literal["group"]
    value: object


class PatchInput(TypedDict):
    kind: Literal["patch"]
    value: object


class SchemaInput(TypedDict):
    kind: Literal["schema"]
    value: object


class ResourceTypeInput(TypedDict):
    kind: Literal["resource_type"]
    value: object


Input = UserInput | GroupInput | PatchInput | SchemaInput | ResourceTypeInput


class Dumpable(Protocol):
    def model_dump(self, **kwargs: object) -> dict[str, object]: ...


def run(input_value: Input) -> dict[str, object]:
    value = input_value["value"]
    if input_value["kind"] == "user":
        return cast(Dumpable, SCIMUser.model_validate(value)).model_dump()
    if input_value["kind"] == "group":
        return cast(Dumpable, SCIMGroup.model_validate(value)).model_dump()
    if input_value["kind"] == "patch":
        return cast(Dumpable, SCIMPatchOp.model_validate(value)).model_dump()
    if input_value["kind"] == "schema":
        return cast(Dumpable, SCIMSchema.model_validate(value)).model_dump()
    return cast(Dumpable, SCIMResourceType.model_validate(value)).model_dump()
