from pydantic import BaseModel, JsonValue


class ComplianceCheckResult(BaseModel):
    check_name: str
    article: str
    passed: bool
    detail: str


class ComplianceResponse(BaseModel):
    compliant: bool
    regulation: str
    checks: list[ComplianceCheckResult]


class ComplianceCheckRequest(BaseModel):
    request_id: str
    user_id: str | None = None
    model: str | None = None
    timestamp: str | None = None
    guardrail_information: list[dict[str, JsonValue]] | None = None
