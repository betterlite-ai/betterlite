import json
from datetime import datetime
from typing import Any, cast

from litellm.models.budget import LiteLLM_BudgetTable
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    row = cast(dict[str, Any], input_value["row"])
    if operation == "budget":
        return cast(Json, json.loads(LiteLLM_BudgetTable.model_validate(row).model_dump_json()))
    if operation == "permission":
        permission = LiteLLM_ObjectPermissionTable.model_validate(row)
        return cast(Json, json.loads(permission.model_dump_json()))
    if operation == "effective":
        budget = LiteLLM_BudgetTable.model_validate(row)
        now = datetime.fromisoformat(cast(str, input_value["now"]))
        return {
            "active_temp_budget_increase": budget.active_temp_budget_increase(now),
            "effective_max_budget": budget.effective_max_budget(now),
        }
    raise ValueError("unsupported operation")
