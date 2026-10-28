# SSE parser choice for B005

The parser must buffer events split across network reads, accept arbitrary bytes
in the fuzz target without panicking, and cap buffered input before allocating.
It must feed payload bytes into the typed chat chunk decoder, including invalid
UTF-8 inputs that the decoder rejects.

Compared [eventsource-stream 0.2.3](https://github.com/jpopesculian/eventsource-stream/blob/v0.2.3/src/event_stream.rs)
with the existing `SseParser` in `src/stream.rs`. The crate accepts byte streams,
buffers split reads, and reports invalid UTF-8 as an error, so it can be fuzzed.
However, its private string buffer and event builder have no configurable byte
limit. Limiting each network read would not bound an event accumulated across
reads; enforcing that externally would still require framing logic.

Retain the hand-written byte parser. It checks the combined pending and incoming
byte count against 1 MiB before extending the buffer, preserves payload bytes,
and buffers incomplete events across reads. Existing split-event, limit,
invalid-UTF-8 relay, and corpus-replay tests cover these requirements. The limit
also rejects a single read larger than 1 MiB even if it contains multiple events.
This is an OpenAI data-event parser, not a browser EventSource implementation;
reconnection, event IDs, and retry directives are not needed by this relay.
