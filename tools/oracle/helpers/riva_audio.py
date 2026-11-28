import sys
from collections.abc import Iterator, Sequence
from importlib.abc import MetaPathFinder
from importlib.machinery import ModuleSpec
from io import BytesIO
from pathlib import Path
from types import ModuleType, TracebackType
from typing import Final, Literal, NoReturn, Self, TypedDict

from litellm.llms.nvidia_riva.audio_transcription.audio_utils import (
    FloatArray,
    ResampledAudio,
    resample_to_riva_pcm,
)
from litellm.llms.nvidia_riva.common_utils import NvidiaRivaException
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
MANAGED: Final = ("soundfile", "audioread", "soxr", "scipy", "scipy.signal")

Resampler = Literal["soxr", "scipy"]
Failure = Literal["none", "open", "iteration"]


class AudioreadSpec(TypedDict):
    chunks: list[str]
    samplerate: int
    channels: int
    failure: Failure


class PipelineCall(TypedDict):
    file_hex: str
    soundfile_error: str | None
    audioread: AudioreadSpec | None
    resamplers: list[Resampler]


class Input(TypedDict):
    calls: list[PipelineCall]


class Answer(TypedDict):
    status: int | None
    message: str | None
    pcm_hex: str | None
    sample_rate_hz: int | None
    num_channels: int | None
    duration_seconds: float | None
    events: list[str]
    staging_suffix: str | None
    staging_removed: bool | None


class FakeSoundFile(ModuleType):
    def __init__(self, message: str) -> None:
        super().__init__("soundfile")
        self.message = message

    def read(self, buffer: BytesIO, *, dtype: str, always_2d: bool) -> NoReturn:
        if buffer.closed or dtype != "float32" or always_2d:
            raise ValueError("the fake soundfile reads only an open float32 stream")
        raise RuntimeError(self.message)


class FakeSource:
    def __init__(self, spec: AudioreadSpec, events: list[str]) -> None:
        self.spec = spec
        self.events = events

    @property
    def samplerate(self) -> int:
        return self.spec["samplerate"]

    @property
    def channels(self) -> int:
        return self.spec["channels"]

    def __enter__(self) -> Self:
        self.events.append("audioread.enter")
        return self

    def __exit__(
        self,
        _exception_type: type[BaseException] | None,
        _exception: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.events.append("audioread.exit")

    def __iter__(self) -> Iterator[bytes]:
        if self.spec["failure"] == "iteration":
            raise RuntimeError("cannot read audio")
        for chunk in self.spec["chunks"]:
            self.events.append("audioread.read")
            yield bytes.fromhex(chunk)


class FakeAudioRead(ModuleType):
    def __init__(self, spec: AudioreadSpec, events: list[str], staging: list[str]) -> None:
        super().__init__("audioread")
        self.spec = spec
        self.events = events
        self.staging = staging

    def audio_open(self, path: str) -> FakeSource:
        self.staging.append(path)
        self.events.append("audioread.open")
        if self.spec["failure"] == "open":
            raise RuntimeError("cannot open audio")
        return FakeSource(self.spec, self.events)


class FakeSoXR(ModuleType):
    def __init__(self, events: list[str]) -> None:
        super().__init__("soxr")
        self.events = events

    def resample(self, samples: FloatArray, source_rate: int, target_rate: int) -> FloatArray:
        self.events.append(f"soxr.resample:{source_rate}:{target_rate}")
        return samples


class FakeScipySignal(ModuleType):
    def __init__(self, events: list[str]) -> None:
        super().__init__("scipy.signal")
        self.events = events

    def resample_poly(self, samples: FloatArray, up: int, down: int) -> FloatArray:
        self.events.append(f"scipy.resample_poly:{up}:{down}")
        return samples


class FakeScipy(ModuleType):
    def __init__(self, signal: FakeScipySignal) -> None:
        super().__init__("scipy")
        self.signal = signal


class MissingModules(MetaPathFinder):
    def __init__(self, names: tuple[str, ...]) -> None:
        self.names = names

    @override
    def find_spec(
        self,
        fullname: str,
        _path: Sequence[str] | None,
        _target: ModuleType | None = None,
    ) -> ModuleSpec | None:
        if fullname in self.names:
            raise ImportError(f"{fullname} is hidden for this oracle case")
        return None


def restore_modules(saved: dict[str, ModuleType | None]) -> None:
    for name, module in saved.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module


def managed_names(call: PipelineCall) -> tuple[str, ...]:
    names: list[str] = [] if call["audioread"] is not None else ["audioread"]
    if "soxr" not in call["resamplers"]:
        names.append("soxr")
    if "scipy" not in call["resamplers"]:
        names.extend(("scipy", "scipy.signal"))
    return tuple(names)


def install_call(call: PipelineCall, events: list[str], staging: list[str]) -> None:
    for name in MANAGED:
        sys.modules.pop(name, None)
    soundfile_error = call["soundfile_error"]
    if soundfile_error is not None:
        sys.modules["soundfile"] = FakeSoundFile(soundfile_error)
    audioread = call["audioread"]
    if audioread is not None:
        sys.modules["audioread"] = FakeAudioRead(audioread, events, staging)
    if "soxr" in call["resamplers"]:
        sys.modules["soxr"] = FakeSoXR(events)
    if "scipy" in call["resamplers"]:
        signal = FakeScipySignal(events)
        sys.modules["scipy.signal"] = signal
        sys.modules["scipy"] = FakeScipy(signal)


def staging_state(staging: list[str]) -> tuple[str | None, bool | None]:
    if not staging:
        return (None, None)
    path = Path(staging[-1])
    return (path.suffix.removeprefix("."), not path.exists())


def empty_answer(events: list[str], staging: list[str]) -> Answer:
    suffix, removed = staging_state(staging)
    return {
        "status": None,
        "message": None,
        "pcm_hex": None,
        "sample_rate_hz": None,
        "num_channels": None,
        "duration_seconds": None,
        "events": events,
        "staging_suffix": suffix,
        "staging_removed": removed,
    }


def success_answer(audio: ResampledAudio, events: list[str], staging: list[str]) -> Answer:
    answer = empty_answer(events, staging)
    answer["pcm_hex"] = audio.pcm_bytes.hex()
    answer["sample_rate_hz"] = audio.sample_rate_hz
    answer["num_channels"] = audio.num_channels
    answer["duration_seconds"] = audio.duration_seconds
    return answer


def failure_answer(error: NvidiaRivaException, events: list[str], staging: list[str]) -> Answer:
    answer = empty_answer(events, staging)
    answer["status"] = error.status_code
    answer["message"] = error.message
    return answer


def convert(file_hex: str, events: list[str], staging: list[str]) -> Answer:
    try:
        audio = resample_to_riva_pcm(bytes.fromhex(file_hex))
    except NvidiaRivaException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_riva/common_utils.py#L10-L17
        return failure_answer(error, events, staging)
    return success_answer(audio, events, staging)


def execute(call: PipelineCall) -> Answer:
    events: list[str] = []
    staging: list[str] = []
    saved_modules = {name: sys.modules.get(name) for name in MANAGED}
    saved_meta_path = list(sys.meta_path)
    try:
        install_call(call, events, staging)
        sys.meta_path.insert(0, MissingModules(managed_names(call)))
        return convert(call["file_hex"], events, staging)
    finally:
        sys.meta_path[:] = saved_meta_path
        restore_modules(saved_modules)


def run(input_value: Input) -> JsonValue:
    answers = [execute(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
