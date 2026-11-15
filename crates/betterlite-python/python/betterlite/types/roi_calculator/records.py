from typing import Literal, NotRequired

from typing_extensions import ReadOnly, TypedDict


class ROISpendRecord(TypedDict):
    date: ReadOnly[str]
    user_id: ReadOnly[str]
    email: ReadOnly[str]
    spend: ReadOnly[float]
    requests: ReadOnly[int]


class ROIEstimate(TypedDict):
    status: ReadOnly[Literal["estimated", "needs_review", "error"]]
    hours: ReadOnly[float | None]
    reasoning: ReadOnly[str]
    model: NotRequired[ReadOnly[str]]
    evidence_source: NotRequired[ReadOnly[str]]
    effort_basis: NotRequired[ReadOnly[str]]
    cached: NotRequired[ReadOnly[bool]]


class ROIPullRecord(TypedDict):
    repo: ReadOnly[str]
    number: ReadOnly[int]
    title: ReadOnly[str]
    url: ReadOnly[str]
    login: ReadOnly[str]
    emails: ReadOnly[tuple[str, ...]]
    profile_email: ReadOnly[str]
    commit_emails: NotRequired[ReadOnly[tuple[str, ...]]]
    merged_at: ReadOnly[str]
    head_sha: ReadOnly[str]
    additions: ReadOnly[int]
    deletions: ReadOnly[int]
    changed_files: ReadOnly[int]
    commit_count: ReadOnly[int]
    incomplete_metadata: ReadOnly[bool]
    estimate: ReadOnly[ROIEstimate]
    cache_key: ReadOnly[str | None]


class ROIReport(TypedDict):
    mode: ReadOnly[str]
    start: ReadOnly[str]
    end: ReadOnly[str]
    synced_at: ReadOnly[str]
    repos: ReadOnly[tuple[str, ...]]
    estimator_model: ReadOnly[str]
    estimator_prompt: ReadOnly[str]
    effort_basis: ReadOnly[str]
    spend: ReadOnly[tuple[ROISpendRecord, ...]]
    pulls: ReadOnly[tuple[ROIPullRecord, ...]]
    settings_fingerprint: ReadOnly[str]
    warnings: NotRequired[ReadOnly[tuple[str, ...]]]
    unavailable_repos: NotRequired[ReadOnly[tuple[str, ...]]]
    id: NotRequired[ReadOnly[str]]


class ROIPullFile(TypedDict):
    filename: ReadOnly[str | None]
    status: ReadOnly[str | None]
    additions: ReadOnly[int | None]
    deletions: ReadOnly[int | None]


class ROIPullCommit(TypedDict):
    sha: ReadOnly[str]
    message: ReadOnly[str]
    additions: NotRequired[ReadOnly[int]]
    deletions: NotRequired[ReadOnly[int]]
    changed_files: NotRequired[ReadOnly[int | None]]


class ROIPullEvidence(TypedDict):
    repo: ReadOnly[str]
    number: ReadOnly[int]
    title: ReadOnly[str]
    body: ReadOnly[str]
    url: ReadOnly[str]
    login: ReadOnly[str]
    emails: ReadOnly[tuple[str, ...]]
    profile_email: ReadOnly[str]
    commit_emails: NotRequired[ReadOnly[tuple[str, ...]]]
    merged_at: ReadOnly[str]
    head_sha: ReadOnly[str]
    additions: ReadOnly[int]
    deletions: ReadOnly[int]
    changed_files: ReadOnly[int]
    files: ReadOnly[tuple[ROIPullFile, ...]]
    commits: ReadOnly[tuple[ROIPullCommit, ...]]
    commit_count: ReadOnly[int]
    incomplete_metadata: ReadOnly[bool]


class ROIIdentityMatch(TypedDict):
    email: ReadOnly[str]
    match_method: ReadOnly[str]
    matched: ReadOnly[bool]


class ROIPersonSummary(TypedDict):
    id: ReadOnly[str]
    email: ReadOnly[str]
    logins: ReadOnly[tuple[str, ...]]
    spend: ReadOnly[float | None]
    hours: ReadOnly[float]
    prs: ReadOnly[int]
    estimated_prs: ReadOnly[int]
    pending_prs: ReadOnly[int]
    match_methods: ReadOnly[tuple[str, ...]]
    eligible: ReadOnly[bool]
    cost_per_hour: ReadOnly[float | None]


class ROIPullSummary(TypedDict):
    repo: ReadOnly[str]
    number: ReadOnly[int]
    title: ReadOnly[str]
    url: ReadOnly[str]
    login: ReadOnly[str]
    emails: ReadOnly[tuple[str, ...]]
    profile_email: ReadOnly[str]
    merged_at: ReadOnly[str]
    head_sha: ReadOnly[str]
    additions: ReadOnly[int]
    deletions: ReadOnly[int]
    changed_files: ReadOnly[int]
    commit_count: ReadOnly[int]
    incomplete_metadata: ReadOnly[bool]
    estimate: ReadOnly[ROIEstimate]
    cache_key: ReadOnly[str | None]
    email: ReadOnly[str]
    match_method: ReadOnly[str]
    matched: ReadOnly[bool]


class ROISummaryMetrics(TypedDict):
    matched_spend: ReadOnly[float]
    output_hours: ReadOnly[float]
    total_spend: ReadOnly[float]
    total_output_hours: ReadOnly[float]
    excluded_spend: ReadOnly[float]
    cost_per_hour: ReadOnly[float | None]
    hours_per_dollar: ReadOnly[float | None]
    merged_prs: ReadOnly[int]
    estimated_prs: ReadOnly[int]
    matched_prs: ReadOnly[int]
    cohort_people: ReadOnly[int]
    people_with_prs: ReadOnly[int]
    pending_prs: ReadOnly[int]


class ROITrendDay(TypedDict):
    date: ReadOnly[str]
    spend: ReadOnly[float]
    hours: ReadOnly[float]
    prs: ReadOnly[int]


class ROISummary(TypedDict):
    id: ReadOnly[str | None]
    mode: ReadOnly[str]
    start: ReadOnly[str]
    end: ReadOnly[str]
    synced_at: ReadOnly[str]
    repos: ReadOnly[tuple[str, ...]]
    estimator_model: ReadOnly[str]
    estimator_prompt: ReadOnly[str]
    warnings: ReadOnly[tuple[str, ...]]
    effort_basis: ReadOnly[str | None]
    metrics: ReadOnly[ROISummaryMetrics]
    people: ReadOnly[tuple[ROIPersonSummary, ...]]
    pulls: ReadOnly[tuple[ROIPullSummary, ...]]
    trend: ReadOnly[tuple[ROITrendDay, ...]]
