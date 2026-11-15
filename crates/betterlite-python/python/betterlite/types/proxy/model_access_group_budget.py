from pydantic import BaseModel


class ModelAccessGroupBudget(BaseModel):
    access_group_name: str
    spend: float = 0.0
    max_budget: float | None = None
