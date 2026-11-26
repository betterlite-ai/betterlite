from typing import cast

from litellm.models.budget import LiteLLM_BudgetTable
from litellm.proxy.auth import user_api_key_auth
from pydantic import JsonValue

from .case import auth_of
from .doubles import Call


def end_user_apply_call(call: Call) -> JsonValue:
    budget = call.get("budget")
    if budget is None:
        raise ValueError("an end user apply call names the budget table")
    budget_info = LiteLLM_BudgetTable.model_validate(budget)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/budget.py#L16
    params: dict[str, JsonValue] = {}
    user_api_key_auth._apply_budget_limits_to_end_user_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L613-L641
        params, budget_info, call.get("end_user_id")
    )
    return {
        "tpm": params.get("end_user_tpm_limit"),
        "rpm": params.get("end_user_rpm_limit"),
        "tpd": params.get("end_user_tpd_limit"),
        "max_budget": params.get("end_user_max_budget"),
        "model_max_budget": params.get("end_user_model_max_budget"),
    }


def end_user_update_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    params = call.get("params")
    if params is None:
        raise ValueError("an end user update call names the params")
    updated = user_api_key_auth.update_valid_token_with_end_user_params(fixed, params)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L710-L726
    return {
        "end_user_id": updated.end_user_id,
        "tpm": updated.end_user_tpm_limit,
        "rpm": updated.end_user_rpm_limit,
        "tpd": updated.end_user_tpd_limit,
        "max_budget": updated.end_user_max_budget,
        "model_max_budget": cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3315
            "dict[str, JsonValue] | None", updated.end_user_model_max_budget
        ),
    }
