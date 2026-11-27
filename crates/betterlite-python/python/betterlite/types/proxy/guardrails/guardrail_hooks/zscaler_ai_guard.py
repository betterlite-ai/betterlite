import logging
import os
from typing import Final

from pydantic import BaseModel, Field, model_validator
from typing_extensions import Self, override

from betterlite.types.guardrails.pii import GuardrailParamUITypes
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel

LOGGER: Final = logging.getLogger("LiteLLM")

DEFAULT_API_BASE: Final = "https://api.us1.zseclipse.net/v1/detection/execute-policy"
MIN_POLICY_ID: Final = 1


class ZscalerAIGuardConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = Field(
        default=None,
        json_schema_extra={
            "examples": [
                "https://api.us1.zseclipse.net/v1/detection/execute-policy",
                "https://api.us1.zseclipse.net/v1/detection/resolve-and-execute-policy",
            ],
        },
    )
    policy_id: int | None = Field(
        default=None,
        json_schema_extra={
            "ui_hint": "conditional_required",
            "condition": "Required when api_base ends with /execute-policy",
        },
    )
    send_user_api_key_alias: bool | None = Field(
        default=False,
        json_schema_extra={
            "ui_type": GuardrailParamUITypes.BOOL.value,
            "ui_hint": "recommended_when",
            "condition": "Recommended when api_base ends with /resolve-and-execute-policy",
        },
    )
    send_user_api_key_user_id: bool | None = Field(
        default=False,
        json_schema_extra={"ui_type": GuardrailParamUITypes.BOOL.value},
    )
    send_user_api_key_team_id: bool | None = Field(
        default=False,
        json_schema_extra={"ui_type": GuardrailParamUITypes.BOOL.value},
    )
    timeout: float | None = None

    @model_validator(mode="after")
    # ast-grep-ignore: no-long-names
    def validate_endpoint_configuration(self) -> Self:
        env_api_base: Final = os.getenv("ZSCALER_AI_GUARD_URL")
        api_base: Final = (
            self.api_base
            if self.api_base is not None
            else (env_api_base or DEFAULT_API_BASE)
        )
        policy_id: int | None = self.policy_id
        if policy_id is None:
            env_policy: Final = os.getenv("ZSCALER_AI_GUARD_POLICY_ID")
            if env_policy:
                try:
                    policy_id = int(env_policy)
                except ValueError:
                    LOGGER.warning(
                        "ZSCALER_AI_GUARD_POLICY_ID env var is not a valid integer: %s",
                        env_policy,
                    )
        is_resolve_policy: Final = api_base.endswith("/resolve-and-execute-policy")
        is_execute_policy: Final = (
            api_base.endswith("/execute-policy") and not is_resolve_policy
        )
        if is_execute_policy and (policy_id is None or policy_id < MIN_POLICY_ID):
            LOGGER.warning(
                "Using /execute-policy endpoint without a valid policy_id. "
                "Ensure ZSCALER_AI_GUARD_POLICY_ID environment variable is set, "
                "or provide policy_id via request/key/team metadata.",
            )
        if is_resolve_policy and not self.send_user_api_key_alias:
            LOGGER.warning(
                "Using /resolve-and-execute-policy endpoint without send_user_api_key_alias=true. "
                "The endpoint requires user-api-key-alias header to resolve the policy. "
                "Set send_user_api_key_alias to true or ensure the header is sent via other means.",
            )
        return self

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Zscaler AI Guard"
