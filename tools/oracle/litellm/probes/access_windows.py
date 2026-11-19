from datetime import datetime, time, timezone
from types import SimpleNamespace
from typing import ClassVar, Final, Literal

from litellm.exceptions import BadRequestError
from litellm.router_utils.access_windows import filter_reserved_deployments
from litellm.router_utils.common_utils import filter_team_based_models
from pydantic import BaseModel, ConfigDict, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

BAD_REQUEST_PREFIX: Final = "litellm.BadRequestError: "


class Window(Strict):
    start: str
    end: str
    timezone: str
    team_ids: list[str]


class Deployment(Strict):
    model_name: str
    id: str
    team_id: str | None = None
    team_public_model_name: str | None = None
    access_windows: list[Window] | None = None


class Output(Strict):
    kept: list[str]
    error: str | None


class Info(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    id: str


class Returned(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_info: Info


class BlockingWindow(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", from_attributes=True)
    end: time
    timezone: str


class Reservation(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", from_attributes=True)
    deployments: list[Returned]
    blocking_window: BlockingWindow | None


SHAPED: Final = TypeAdapter(list[Returned])


def entry(deployment: Deployment) -> dict[str, object]:
    model_info: dict[str, object] = {"id": deployment.id}
    if deployment.team_id is not None:
        model_info["team_id"] = deployment.team_id
    if deployment.team_public_model_name is not None:
        model_info["team_public_model_name"] = deployment.team_public_model_name
    if deployment.access_windows is not None:
        model_info["access_windows"] = [
            {"start": window.start, "end": window.end, "timezone": window.timezone, "team_ids": window.team_ids}
            for window in deployment.access_windows
        ]
    return {"model_name": deployment.model_name, "model_info": model_info}


def request_kwargs(model: str, team_id: str | None, proxy_admin: bool) -> dict[str, object]:
    metadata: dict[str, object] = {"model_group": model}
    if team_id is not None:
        metadata["user_api_key_team_id"] = team_id
    if proxy_admin:
        metadata["user_api_key_auth"] = SimpleNamespace(user_role="proxy_admin")
    return {"model": model, "metadata": metadata}


def moment(unix_millis: int) -> datetime:
    seconds, millis = divmod(unix_millis, 1000)
    return datetime.fromtimestamp(seconds, tz=timezone.utc).replace(microsecond=millis * 1000)


class AccessWindows(Operation):
    model: str
    deployments: list[Deployment]
    mode: Literal["team", "reserved"] = "team"
    team_id: str | None = None
    proxy_admin: bool = False
    now_unix_millis: int | None = None

    @override
    def run(self) -> Output:
        deployments = [entry(deployment) for deployment in self.deployments]
        if self.mode == "reserved":
            return self.reserved(deployments)
        return self.team(deployments)

    def team(self, deployments: list[dict[str, object]]) -> Output:
        try:
            kept = filter_team_based_models(deployments, request_kwargs(self.model, self.team_id, self.proxy_admin))
        except BadRequestError as error:
            return Output(kept=[], error=str(error.message).removeprefix(BAD_REQUEST_PREFIX))
        return Output(kept=[returned.model_info.id for returned in SHAPED.validate_python(kept)], error=None)

    def reserved(self, deployments: list[dict[str, object]]) -> Output:
        if self.now_unix_millis is None:
            raise ProbeError("invalid_request", "the reserved mode needs now_unix_millis")
        result = Reservation.model_validate(
            filter_reserved_deployments(deployments, self.team_id, moment(self.now_unix_millis)),
            from_attributes=True,
        )
        kept = [returned.model_info.id for returned in result.deployments]
        blocking = result.blocking_window
        if blocking is None or kept:
            return Output(kept=kept, error=None)
        until = blocking.end.strftime("%H:%M")
        message = f"Deployment {self.model} is reserved for another team until {until} {blocking.timezone}"
        return Output(kept=[], error=message)
