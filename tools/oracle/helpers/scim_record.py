from typing import Literal, TypedDict

from litellm.types.proxy.management_endpoints.scim_v2 import (
    SCIMGroup,
    SCIMPatchOp,
    SCIMResourceType,
    SCIMSchema,
    SCIMUser,
)


class Input(TypedDict):
    kind: Literal["user", "group", "patch", "schema", "resource_type"]
    value: dict[str, object]


def run(input_value: Input) -> object:
    value = input_value["value"]
    match input_value["kind"]:
        case "user":
            user = SCIMUser.model_validate(value)
            # FastAPI answers with response_model=SCIMUser, which dumps by alias
            return {"record": user.model_dump(by_alias=True), "active_set": "active" in user.model_fields_set}
        case "group":
            return SCIMGroup.model_validate(value).model_dump()
        case "patch":
            return SCIMPatchOp.model_validate(value).model_dump()
        case "schema":
            return SCIMSchema.model_validate(value).model_dump()
        case "resource_type":
            return SCIMResourceType.model_validate(value).model_dump()
