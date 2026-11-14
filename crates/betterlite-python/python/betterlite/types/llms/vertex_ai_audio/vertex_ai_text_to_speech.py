from typing_extensions import TypedDict


class VertexTextToSpeechInput(TypedDict, total=False):
    text: str | None
    ssml: str | None


class VertexTextToSpeechVoice(TypedDict, total=False):
    languageCode: str
    name: str


class VertexTextToSpeechAudioConfig(TypedDict, total=False):
    audioEncoding: str
    speakingRate: str


class VertexTextToSpeechRequest(TypedDict, total=False):
    input: VertexTextToSpeechInput
    voice: VertexTextToSpeechVoice
    audioConfig: VertexTextToSpeechAudioConfig | None
