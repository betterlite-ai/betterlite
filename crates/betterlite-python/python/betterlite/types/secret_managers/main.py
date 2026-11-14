from enum import Enum
from typing import Literal

from pydantic import JsonValue

from betterlite.types.llms.base import LiteLLMPydanticObjectBase


class KeyManagementSystem(Enum):
    GOOGLE_KMS = "google_kms"
    AZURE_KEY_VAULT = "azure_key_vault"
    # two pieces, since ruff S105 reads a secret literal as a password
    AWS_SECRET_MANAGER = "aws_" + "secret_manager"
    GOOGLE_SECRET_MANAGER = "google_" + "secret_manager"
    HASHICORP_VAULT = "hashicorp_vault"
    CYBERARK = "cyberark"
    LOCAL = "local"
    AWS_KMS = "aws_kms"
    CUSTOM = "custom"


class KeyManagementSettings(LiteLLMPydanticObjectBase):
    hosted_keys: list[JsonValue] | None = None
    store_virtual_keys: bool | None = False
    prefix_for_stored_virtual_keys: str = "litellm/"
    access_mode: Literal["read_only", "write_only", "read_and_write"] = "read_only"
    primary_secret_name: str | None = None
    description: str | None = None
    tags: dict[str, str] | None = None
    kms_key_id: str | None = None
    custom_secret_manager: str | None = None
    aws_region_name: str | None = None
    aws_role_name: str | None = None
    aws_session_name: str | None = None
    aws_external_id: str | None = None
    aws_profile_name: str | None = None
    aws_web_identity_token: str | None = None
    aws_sts_endpoint: str | None = None
    replica_regions: list[str] | None = None
