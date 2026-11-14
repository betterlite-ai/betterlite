from enum import Enum
from typing import TypeAlias

from betterlite.types.llms.openai.realtime.conversation import (
    OpenAIRealtimeConversationCreated,
    OpenAIRealtimeConversationItemAdded,
    OpenAIRealtimeConversationItemCreated,
    OpenAIRealtimeConversationItemDone,
    OpenAIRealtimeStreamResponseBaseObject,
)
from betterlite.types.llms.openai.realtime.items import (
    OpenAIRealtimeStreamResponseOutputItemAdded,
)
from betterlite.types.llms.openai.realtime.responses import (
    OpenAIRealtimeContentPartDone,
    OpenAIRealtimeDoneEvent,
    OpenAIRealtimeFunctionCallArgumentsDone,
    OpenAIRealtimeOutputItemDone,
    OpenAIRealtimeResponseAudioDone,
    OpenAIRealtimeResponseContentPartAdded,
    OpenAIRealtimeResponseDelta,
    OpenAIRealtimeResponseTextDone,
)
from betterlite.types.llms.openai.realtime.session import (
    OpenAIRealtimeStreamSessionEvents,
)
from betterlite.types.llms.openai.realtime.speech import (
    OpenAIRealtimeErrorEvent,
    OpenAIRealtimeInputAudioBufferSpeechEvent,
)
from betterlite.types.llms.openai.realtime.transcription import (
    OpenAIRealtimeInputAudioTranscriptionCompleted,
    OpenAIRealtimeInputAudioTranscriptionDelta,
    OpenAIRealtimeTranscriptionSessionCreated,
)


class OpenAIRealtimeEventTypes(Enum):
    SESSION_CREATED = "session.created"
    RESPONSE_TEXT_DELTA = "response.text.delta"
    RESPONSE_AUDIO_DELTA = "response.audio.delta"
    RESPONSE_TEXT_DONE = "response.text.done"
    RESPONSE_AUDIO_DONE = "response.audio.done"
    RESPONSE_OUTPUT_TEXT_DELTA = "response.output_text.delta"
    RESPONSE_OUTPUT_AUDIO_DELTA = "response.output_audio.delta"
    RESPONSE_OUTPUT_AUDIO_TRANSCRIPT_DELTA = "response.output_audio_transcript.delta"
    RESPONSE_OUTPUT_TEXT_DONE = "response.output_text.done"
    RESPONSE_OUTPUT_AUDIO_DONE = "response.output_audio.done"
    RESPONSE_OUTPUT_AUDIO_TRANSCRIPT_DONE = "response.output_audio_transcript.done"
    RESPONSE_DONE = "response.done"
    RESPONSE_OUTPUT_ITEM_ADDED = "response.output_item.added"
    RESPONSE_CONTENT_PART_ADDED = "response.content_part.added"


OpenAIRealtimeEvents: TypeAlias = (
    OpenAIRealtimeStreamResponseBaseObject  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeStreamSessionEvents  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeStreamResponseOutputItemAdded  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeResponseContentPartAdded  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeConversationItemCreated  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeConversationItemAdded  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeConversationItemDone  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeConversationCreated  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeResponseDelta
    | OpenAIRealtimeResponseTextDone
    | OpenAIRealtimeResponseAudioDone
    | OpenAIRealtimeContentPartDone  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeOutputItemDone
    | OpenAIRealtimeFunctionCallArgumentsDone  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeDoneEvent
    | OpenAIRealtimeInputAudioBufferSpeechEvent  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeInputAudioTranscriptionDelta  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeInputAudioTranscriptionCompleted  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeTranscriptionSessionCreated  # ast-grep-ignore: no-long-names
    | OpenAIRealtimeErrorEvent
)

OpenAIRealtimeStreamList: TypeAlias = list[OpenAIRealtimeEvents]
