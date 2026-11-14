from collections.abc import Mapping
from typing import Literal

from pydantic import BaseModel

class ROIMetricsResponse(BaseModel):
    matched_spend: float
    output_hours: float
    total_spend: float
    total_output_hours: float
    excluded_spend: float
    cost_per_hour: float | None
    hours_per_dollar: float | None
    merged_prs: int
    estimated_prs: int
    matched_prs: int
    cohort_people: int
    people_with_prs: int
    pending_prs: int


class ROIPersonResponse(BaseModel):
    id: str
    email: str
    logins: tuple[str, ...]
    spend: float | None
    hours: float
    prs: int
    estimated_prs: int
    pending_prs: int
    match_methods: tuple[str, ...]
    eligible: bool
    cost_per_hour: float | None


class ROIEstimateResponse(BaseModel):
    status: Literal["estimated", "needs_review", "error"]
    hours: float | None
    reasoning: str
    model: str | None = None
    evidence_source: str | None = None
    effort_basis: str | None = None
    cached: bool = False


class ROIPullResponse(BaseModel):
    repo: str
    number: int
    title: str
    url: str
    login: str
    emails: tuple[str, ...]
    profile_email: str
    merged_at: str
    head_sha: str
    additions: int
    deletions: int
    changed_files: int
    commit_count: int
    incomplete_metadata: bool
    estimate: ROIEstimateResponse
    cache_key: str | None = None
    email: str
    match_method: str
    matched: bool


class ROITrendResponse(BaseModel):
    date: str
    spend: float
    hours: float
    prs: int


class ROISummaryResponse(BaseModel):
    id: str | None
    mode: str
    start: str
    end: str
    synced_at: str
    repos: tuple[str, ...]
    estimator_model: str
    estimator_prompt: str
    warnings: tuple[str, ...]
    effort_basis: str | None
    metrics: ROIMetricsResponse
    people: tuple[ROIPersonResponse, ...]
    pulls: tuple[ROIPullResponse, ...]
    trend: tuple[ROITrendResponse, ...]


class ROIReportResponse(BaseModel):
    report: ROISummaryResponse | None


class ROIIdentityMapResponse(BaseModel):
    report: ROISummaryResponse | None
    identity_map: Mapping[str, str]
