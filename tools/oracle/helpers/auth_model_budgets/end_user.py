from collections.abc import Mapping
from typing import cast

import litellm
from litellm.models.budget import LiteLLM_BudgetTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import user_api_key_auth
from pydantic import JsonValue

litellm.suppress_debug_info = True

END_USER_LIMIT_FIELDS = {
    "tpm": "end_user_tpm_limit",
    "rpm": "end_user_rpm_limit",
    "tpd": "end_user_tpd_limit",
    "max_budget": "end_user_max_budget",
    "model_max_budget": "end_user_model_max_budget",
}


def auth_of(call: Mapping[str, JsonValue]) -> UserAPIKeyAuth:
    token = call.get("token")
    if token is None:
        raise ValueError("the call names the token fields")
    return UserAPIKeyAuth.model_validate(token)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def end_user_result(
    end_user_id: JsonValue, values: Mapping[str, JsonValue]
) -> JsonValue:
    return {
        "end_user_id": end_user_id,
        "tpm": values.get("tpm"),
        "rpm": values.get("rpm"),
        "tpd": values.get("tpd"),
        "max_budget": values.get("max_budget"),
        "model_max_budget": values.get("model_max_budget"),
    }


def end_user_apply_call(call: Mapping[str, JsonValue]) -> JsonValue:
    budget = call.get("budget")
    if budget is None:
        raise ValueError("an end user apply call names the budget table")
    budget_info = LiteLLM_BudgetTable.model_validate(budget)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/budget.py#L16
    limits: dict[str, JsonValue] = {}
    user_api_key_auth._apply_budget_limits_to_end_user_params(limits, budget_info, call.get("end_user_id"))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L613-L641
    values = {
        output: limits.get(source) for output, source in END_USER_LIMIT_FIELDS.items()
    }
    return end_user_result(limits.get("end_user_id"), values)


def end_user_update_call(call: Mapping[str, JsonValue]) -> JsonValue:
    params = call.get("params")
    if params is None:
        raise ValueError("an end user update call names the params")
    updated = user_api_key_auth.update_valid_token_with_end_user_params(auth_of(call), params)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L710-L726
    model_max_budget = cast("JsonValue", updated.end_user_model_max_budget)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3315
    values = {
        "tpm": updated.end_user_tpm_limit,
        "rpm": updated.end_user_rpm_limit,
        "tpd": updated.end_user_tpd_limit,
        "max_budget": updated.end_user_max_budget,
        "model_max_budget": model_max_budget,
    }
    return end_user_result(updated.end_user_id, values)
