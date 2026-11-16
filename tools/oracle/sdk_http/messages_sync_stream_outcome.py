from pydantic import BaseModel, ConfigDict
from pydantic import JsonValue as Json


class State(BaseModel):
    model_config = ConfigDict(extra="forbid")

    pulls: int
    index: int
    block_type: str
    sent_first_chunk: bool
    sent_content_block_start: bool
    sent_content_block_finish: bool
    sent_last_message: bool
    queued_usage_chunk: bool
    holding_stop_reason: bool
    sent_compaction_block: bool
    sent_compaction_block_start: bool
    sent_compaction_block_delta: bool


class Diagnostic(BaseModel):
    model_config = ConfigDict(extra="forbid")

    level: str
    prefix: str


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    events: list[dict[str, Json]]
    end: bool
    state: State
    diagnostics: list[Diagnostic]


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[StepOutcome]


class SyncValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class SyncOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: SyncValue


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_sync_stream: SyncOutcome
