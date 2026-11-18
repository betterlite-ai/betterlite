import asyncio
import os
import sys

from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.realtime.handler import BedrockRealtime
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from pydantic import JsonValue

from bedrock_services_realtime_session import make_logging

from .client import CredentialResolver, DiscardingWorker, FakeWebSocket
from .fakes import (
    CLIENTS,
    RECORDING,
    SCRIPT,
    SDK_MODULES,
    ClientSendError,
    CredentialLookupError,
    Script,
    StreamOpenError,
)
from .models import (
    BAD_REQUEST,
    COMMITTED_KEY,
    COMMITTED_PREFIX,
    FAILURE_KEY,
    SUCCESS_KEY,
    TRACE_ID,
    UNAUTHORIZED,
    ClientMessageAdapter,
    Input,
    InputFrameAdapter,
    JsonAdapter,
    Run,
    ScenarioView,
)


def bedrock_error_kind(error: BedrockError) -> tuple[str, int]:
    status = error.status_code
    if status == BAD_REQUEST and error.message.startswith(COMMITTED_PREFIX):
        return ("committed", BAD_REQUEST)
    if status == UNAUTHORIZED:
        return ("no_credentials", UNAUTHORIZED)
    return ("provider_bedrock", status)


def bedrock_key(frame: JsonValue) -> str:
    parsed = InputFrameAdapter.validate_python(frame)
    event = parsed.get("event")
    if not event:
        return ""
    return next(iter(event))


async def run_scenario(run: Run) -> ScenarioView:
    RECORDING.endpoint = None
    RECORDING.region = None
    RECORDING.access_key_id = None
    RECORDING.secret_access_key = None
    RECORDING.session_token = None
    RECORDING.model_ids = []
    RECORDING.resolver_static = False
    RECORDING.transport_injected = False
    SCRIPT.append(Script(run))
    CLIENTS.clear()
    sys.modules.update(SDK_MODULES)
    websocket = FakeWebSocket(run)
    logging = make_logging(TRACE_ID)
    realtime = BedrockRealtime()
    auth = run["auth"]
    BedrockRealtime.resolve_credentials = CredentialResolver(run)  # type: ignore[method-assign, assignment]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L242-L280
    GLOBAL_LOGGING_WORKER.ensure_initialized_and_enqueue = DiscardingWorker()  # type: ignore[method-assign, assignment, attr-defined]  # noqa: E501  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/logging_worker.py#L448-L453
    os.environ.clear()
    error_kind: str | None = None
    status_code: int | None = None
    try:
        await realtime.async_realtime(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L242-L280
            model=run["model"],
            websocket=websocket,  # type: ignore[arg-type]  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L89-L98
            logging_obj=logging,
            api_base=run["api_base"],
            aws_region_name=run["region"],
            aws_bedrock_runtime_endpoint=run["endpoint"],
            aws_access_key_id=auth["access_key_id"] if auth is not None else None,
            aws_secret_access_key=auth["secret_access_key"] if auth is not None else None,
            aws_session_token=auth["session_token"] if auth is not None else None,
        )
    except BedrockError as error:
        error_kind, status_code = bedrock_error_kind(error)
    except StreamOpenError:
        error_kind = "stream"
    except ClientSendError:
        error_kind = "client"
    except CredentialLookupError:
        error_kind = "credential"
    except ValueError:
        error_kind = "endpoint"
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L242-L346
        error_kind = "provider_unchanged"
    script = SCRIPT[-1]
    details: dict[str, JsonValue] = logging.model_call_details
    return ScenarioView(
        endpoint=RECORDING.endpoint,
        region=RECORDING.region,
        access_key_id=RECORDING.access_key_id,
        secret_access_key=RECORDING.secret_access_key,
        session_token=RECORDING.session_token,
        model_ids=list(RECORDING.model_ids),
        client_types=[
            ClientMessageAdapter.validate_json(message)["type"] for message in websocket.sent
        ],
        bedrock_keys=[bedrock_key(frame) for frame in script.sent],
        client_close=websocket.close_code,
        runtime_closed=bool(CLIENTS) and CLIENTS[-1].closed,
        input_closed=script.input_closed,
        resolver_static=RECORDING.resolver_static,
        transport_injected=RECORDING.transport_injected,
        committed=websocket.scope.get(COMMITTED_KEY) is True,
        failure_set=FAILURE_KEY in websocket.scope,
        success_flag=details.get(SUCCESS_KEY) is True,
        outcome="ok" if error_kind is None else "error",
        error_kind=error_kind,
        status_code=status_code,
    )


async def run_all(input_value: Input) -> list[JsonValue]:
    views = [await run_scenario(run) for run in input_value["runs"]]
    return [JsonAdapter.validate_python(view) for view in views]
