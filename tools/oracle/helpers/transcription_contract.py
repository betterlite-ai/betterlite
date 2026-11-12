from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.base_llm.audio_transcription.transformation import (
    AudioTranscriptionRequestData,
    BaseAudioTranscriptionConfig,
)
from litellm.llms.gemini.audio_transcription.transformation import (
    GeminiAudioTranscriptionConfig as GeminiConfig,
)
from litellm.llms.nvidia_riva.audio_transcription.transformation import (
    NvidiaRivaAudioTranscriptionConfig as RivaConfig,
)
from litellm.llms.xai.audio_transcription.transformation import (
    XAIAudioTranscriptionConfig as XaiConfig,
)
from litellm.types.llms.openai import (
    OpenAIAudioTranscriptionOptionalParams as TranscriptionParam,
)
from litellm.types.utils import FileTypes
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
EMPTY: Object = {}
AUDIO: FileTypes = b"abc"


class DefaultsInput(TypedDict):
    function: Literal["defaults"]
    data_object: Object
    data_bytes: str
    named_files: dict[str, str]
    content_type: str
    api_bases: list[str | None]


class ProviderParametersInput(TypedDict):
    function: Literal["provider_parameters"]
    model: str
    supported: list[TranscriptionParam]
    supported_with_extra: list[TranscriptionParam]
    optional_params: Object
    reserved_params: Object
    openai_params: list[TranscriptionParam]
    other_openai_params: list[TranscriptionParam]


Input: TypeAlias = DefaultsInput | ProviderParametersInput


class DataView(TypedDict):
    kind: Literal["object", "bytes"]
    value: Object | str


class FilesView(TypedDict):
    kind: Literal["absent", "empty", "named"]
    value: NotRequired[Object]


class RecordView(TypedDict):
    data: DataView
    files: FilesView
    content_type: str | None


class CapabilityView(TypedDict):
    native: bool
    subtitles: bool


class CapabilitiesView(TypedDict):
    base: CapabilityView
    native: CapabilityView
    subtitle: CapabilityView


class DefaultsOutput(TypedDict):
    default_record: RecordView
    bytes_record: RecordView
    explicit_record: RecordView
    named_record: RecordView
    capabilities: CapabilitiesView
    urls: list[str]
    request_error: str | None
    response_error: str | None


class ProviderParametersOutput(TypedDict):
    filtered: Object
    openai_changed: Object
    supported_extra: Object
    reserved: Object
    after: Object


class TranscriptionRequestData(Protocol):
    data: Object | bytes
    files: Object | None
    content_type: str | None


class ConfigContract(Protocol):
    def get_provider_specific_params(
        self,
        model: str,
        optional_params: Object,
        openai_params: list[TranscriptionParam],
    ) -> Object: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
    ) -> str: ...


class RivaFactory(Protocol):
    def __call__(self) -> RivaConfig: ...


class XaiFactory(Protocol):
    def __call__(self) -> XaiConfig: ...


class GeminiFactory(Protocol):
    def __call__(self) -> GeminiConfig: ...


RIVA: RivaFactory = RivaConfig
XAI: XaiFactory = XaiConfig
GEMINI: GeminiFactory = GeminiConfig


class FixtureAudioConfig(RivaConfig):
    def __init__(self, supported: list[TranscriptionParam]) -> None:
        self.supported = supported

    @override
    def get_supported_openai_params(self, model: str) -> list[TranscriptionParam]:
        return self.supported


def data_value(data: Object | bytes) -> DataView:
    match data:
        case bytes():
            return {"kind": "bytes", "value": data.decode()}
        case dict():
            return {"kind": "object", "value": data}


def files_value(files: Object | None) -> FilesView:
    if files is None:
        return {"kind": "absent"}
    if not files:
        return {"kind": "empty"}
    return {"kind": "named", "value": files}


def record_value(record: TranscriptionRequestData) -> RecordView:
    return {
        "data": data_value(record.data),
        "files": files_value(record.files),
        "content_type": record.content_type,
    }


def capability_flags(config: BaseAudioTranscriptionConfig) -> CapabilityView:
    return {
        "native": config.has_native_transcription_endpoint,
        "subtitles": config.supports_subtitle_synthesis,
    }


def complete_url(config: ConfigContract, api_base: str | None) -> str:
    return config.get_complete_url(
        api_base=api_base,
        api_key=None,
        model="m",
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )


def provider_params(
    config: ConfigContract,
    model: str,
    optional_params: Object,
    openai_params: list[TranscriptionParam],
) -> Object:
    return config.get_provider_specific_params(
        model=model,
        optional_params=optional_params,
        openai_params=openai_params,
    )


def defaults_report(input_value: DefaultsInput) -> DefaultsOutput:
    base = RIVA()
    try:
        BaseAudioTranscriptionConfig.transform_audio_transcription_request(  # pyright: ignore[reportAbstractUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/audio_transcription/transformation.py#L82-L91
            base,
            model="m",
            audio_file=AUDIO,
            optional_params=EMPTY,
            litellm_params=EMPTY,
        )
        request_error = None
    except NotImplementedError as error:
        request_error = str(error)
    try:
        BaseAudioTranscriptionConfig.transform_audio_transcription_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/audio_transcription/transformation.py#L93-L99
            base,
            raw_response=httpx.Response(200),
        )
        response_error = None
    except NotImplementedError as error:
        response_error = str(error)
    object_data = input_value["data_object"]
    default_record = record_value(AudioTranscriptionRequestData(data=object_data))
    bytes_record = record_value(
        AudioTranscriptionRequestData(data=input_value["data_bytes"].encode())
    )
    explicit_record = record_value(
        AudioTranscriptionRequestData(
            data=object_data, files=EMPTY, content_type=input_value["content_type"]
        )
    )
    named_record = record_value(
        AudioTranscriptionRequestData(
            data=object_data, files=input_value["named_files"], content_type=None
        )
    )
    return {
        "default_record": default_record,
        "bytes_record": bytes_record,
        "explicit_record": explicit_record,
        "named_record": named_record,
        "capabilities": {
            "base": capability_flags(base),
            "native": capability_flags(XAI()),
            "subtitle": capability_flags(GEMINI()),
        },
        "urls": [complete_url(base, api_base) for api_base in input_value["api_bases"]],
        "request_error": request_error,
        "response_error": response_error,
    }


def provider_parameters_report(input_value: ProviderParametersInput) -> ProviderParametersOutput:
    return {
        "filtered": provider_params(
            FixtureAudioConfig(input_value["supported"]),
            input_value["model"],
            input_value["optional_params"],
            input_value["openai_params"],
        ),
        "openai_changed": provider_params(
            FixtureAudioConfig(input_value["supported"]),
            input_value["model"],
            input_value["optional_params"],
            input_value["other_openai_params"],
        ),
        "supported_extra": provider_params(
            FixtureAudioConfig(input_value["supported_with_extra"]),
            input_value["model"],
            input_value["optional_params"],
            input_value["openai_params"],
        ),
        "reserved": provider_params(
            FixtureAudioConfig(input_value["supported"]),
            input_value["model"],
            input_value["reserved_params"],
            input_value["openai_params"],
        ),
        "after": input_value["optional_params"],
    }


def run(input_value: Input) -> DefaultsOutput | ProviderParametersOutput:
    if input_value["function"] == "defaults":
        return defaults_report(input_value)
    return provider_parameters_report(input_value)
