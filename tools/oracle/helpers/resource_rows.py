from typing import Literal, TypedDict

from litellm.models.access_group import LiteLLM_AccessGroupTable
from litellm.models.config import LiteLLM_Config
from litellm.models.credentials import (
    CreateCredentialItem,
    CredentialBase,
    CredentialItem,
    UpdateCredentialItem,
)
from litellm.models.model import LiteLLM_ProxyModelTable
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.project import LiteLLM_ProjectTable
from pydantic import BaseModel

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class AccessGroupInput(TypedDict):
    operation: Literal["access_group"]
    row: dict[str, Json]


class ConfigInput(TypedDict):
    operation: Literal["config"]
    row: dict[str, Json]


class CredentialBaseInput(TypedDict):
    operation: Literal["credential_base"]
    row: dict[str, Json]


class CredentialItemInput(TypedDict):
    operation: Literal["credential_item"]
    row: dict[str, Json]


class CredentialCreateInput(TypedDict):
    operation: Literal["credential_create"]
    row: dict[str, Json]


class CredentialUpdateInput(TypedDict):
    operation: Literal["credential_update"]
    row: dict[str, Json]


class ModelInput(TypedDict):
    operation: Literal["model"]
    row: dict[str, Json]


class OrganizationInput(TypedDict):
    operation: Literal["organization"]
    row: dict[str, Json]


class ProjectInput(TypedDict):
    operation: Literal["project"]
    row: dict[str, Json]


Input = (
    AccessGroupInput
    | ConfigInput
    | CredentialBaseInput
    | CredentialItemInput
    | CredentialCreateInput
    | CredentialUpdateInput
    | ModelInput
    | OrganizationInput
    | ProjectInput
)


_MODELS: dict[str, type[BaseModel]] = {
    "access_group": LiteLLM_AccessGroupTable,
    "config": LiteLLM_Config,
    "credential_base": CredentialBase,
    "credential_item": CredentialItem,
    "credential_create": CreateCredentialItem,
    "credential_update": UpdateCredentialItem,
    "model": LiteLLM_ProxyModelTable,
    "organization": LiteLLM_OrganizationTable,
    "project": LiteLLM_ProjectTable,
}


def run(input_value: Input) -> Json:
    model = _MODELS[input_value["operation"]]
    return model.model_validate(input_value["row"]).model_dump(mode="json")
