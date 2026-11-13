from dataclasses import dataclass
from typing import Final, Literal, Required, TypeAlias, TypedDict

import litellm
from google.auth.credentials import TokenState
from litellm.llms.vertex_ai.vertex_llm_base import VertexBase

litellm.suppress_debug_info = True

StateName: TypeAlias = Literal["fresh", "stale", "invalid"]
Key: TypeAlias = tuple[str | None, str | None]
STATES: Final[dict[StateName, TokenState]] = {
    "fresh": TokenState.FRESH,
    "stale": TokenState.STALE,
    "invalid": TokenState.INVALID,
}
STATE_LABELS: Final[dict[TokenState, StateName]] = {
    TokenState.FRESH: "fresh",
    TokenState.STALE: "stale",
    TokenState.INVALID: "invalid",
}


class CredentialFields(TypedDict, total=False):
    name: Required[str]
    token: str
    state: StateName
    expired: bool
    valid: bool
    quota_project_id: str
    project_id: str


class RecordFields(TypedDict, total=False):
    credential: Required[str]
    credentials: str
    project: str
    cached_project: str
    legacy: bool


class ProbeFields(TypedDict, total=False):
    credentials: str
    project: str
    requested_project: str


class ScenarioFields(TypedDict, total=False):
    credentials: list[CredentialFields]
    records: list[RecordFields]
    probes: list[ProbeFields]


class Input(TypedDict):
    scenarios: list[ScenarioFields]


class CredentialState(TypedDict):
    name: str
    state: StateName


class UnpackedView(TypedDict):
    cached: bool
    project: str | None


class TokenView(TypedDict):
    token: str
    project: str


class UsableView(TypedDict):
    token: str
    project: str
    state: StateName
    cached_project: str | None


class ProbeView(TypedDict):
    unpacked: UnpackedView
    fresh: TokenView | None
    usable: UsableView | None


class ScenarioView(TypedDict):
    credentials: list[CredentialState]
    probes: list[ProbeView]


class Output(TypedDict):
    scenarios: list[ScenarioView]


@dataclass(frozen=True)
class ReportedCredential:
    token: str | None
    token_state: TokenState
    quota_project_id: str | None
    project_id: str | None


@dataclass(frozen=True)
class ExpiredCredential:
    token: str | None
    expired: bool
    valid: bool | None
    quota_project_id: str | None
    project_id: str | None


@dataclass(frozen=True)
class BareCredential:
    token: str | None
    quota_project_id: str | None
    project_id: str | None


Credential: TypeAlias = ReportedCredential | ExpiredCredential | BareCredential
Entry: TypeAlias = tuple[Credential, str | None] | Credential


def build_credential(fields: CredentialFields) -> Credential:
    token = fields.get("token")
    quota = fields.get("quota_project_id")
    project = fields.get("project_id")
    state = fields.get("state")
    if state is not None:
        return ReportedCredential(
            token=token,
            token_state=STATES[state],
            quota_project_id=quota,
            project_id=project,
        )
    expired = fields.get("expired")
    if expired is not None:
        return ExpiredCredential(
            token=token,
            expired=expired,
            valid=fields.get("valid"),
            quota_project_id=quota,
            project_id=project,
        )
    return BareCredential(token=token, quota_project_id=quota, project_id=project)


def build_mapping(
    records: list[RecordFields], credentials: dict[str, Credential]
) -> dict[Key, Entry]:
    mapping: dict[Key, Entry] = {}
    for record in records:
        credential = credentials[record["credential"]]
        key = (record.get("credentials"), record.get("project"))
        if record.get("legacy"):
            mapping[key] = credential
        else:
            mapping[key] = (credential, record.get("cached_project"))
    return mapping


def fresh_view(result: tuple[str, str] | None) -> TokenView | None:
    if result is None:
        return None
    token, project = result
    return TokenView(token=token, project=project)


def probe_row(base: VertexBase, mapping: dict[Key, Entry], probe: ProbeFields) -> ProbeView:
    base._credentials_project_mapping = mapping  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L76-L80
    key = (probe.get("credentials"), probe.get("project"))
    requested = probe.get("requested_project")
    cached, cached_project = base._unpack_cached_credentials(key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L467-L479
    fresh = fresh_view(base._try_get_cached_token(key, requested))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L417-L439
    usable_result = base._try_get_usable_cached_token(key, requested)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L440-L466
    usable: UsableView | None = None
    if usable_result is not None:
        token, project, state, _credential, usable_project = usable_result
        usable = UsableView(
            token=token,
            project=project,
            state=STATE_LABELS[state],
            cached_project=usable_project,
        )
    return ProbeView(
        unpacked=UnpackedView(cached=cached is not None, project=cached_project),
        fresh=fresh,
        usable=usable,
    )


def scenario_row(scenario: ScenarioFields) -> ScenarioView:
    base = VertexBase()
    credential_fields = scenario.get("credentials")
    credentials: dict[str, Credential] = {}
    states: list[CredentialState] = []
    for fields in [] if credential_fields is None else credential_fields:
        credential = build_credential(fields)
        credentials[fields["name"]] = credential
        state = base._get_token_state(credential)  # type: ignore[arg-type]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L50-L63
        states.append(CredentialState(name=fields["name"], state=STATE_LABELS[state]))
    record_fields = scenario.get("records")
    mapping = build_mapping([] if record_fields is None else record_fields, credentials)
    probe_fields = scenario.get("probes")
    probes = [
        probe_row(base, mapping, probe)
        for probe in ([] if probe_fields is None else probe_fields)
    ]
    return ScenarioView(credentials=states, probes=probes)


def run(data: Input) -> Output:
    scenarios: list[ScenarioView] = [
        scenario_row(scenario) for scenario in data["scenarios"]
    ]
    return Output(scenarios=scenarios)
