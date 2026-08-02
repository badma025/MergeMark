import abc
import collections.abc
import modal._utils.task_command_router_client
import modal.client
import modal.stream_type
import typing
import typing_extensions

def _sandbox_logs_iterator(
    sandbox_id: str, file_descriptor: int, last_entry_id: str, client: modal.client._Client
) -> collections.abc.AsyncGenerator[tuple[typing.Optional[bytes], str], None]: ...

T = typing.TypeVar("T")

class _StreamReaderThroughServer(typing.Generic[T]):
    """A StreamReader implementation that reads sandbox logs from the server."""

    _stream: typing.Optional[collections.abc.AsyncGenerator[T, None]]

    def __init__(self, params: _StreamReaderThroughServerParams, text: bool = True, by_line: bool = False) -> None:
        """mdmd:hidden"""
        ...

    @property
    def file_descriptor(self) -> int:
        """Possible values are `1` for stdout and `2` for stderr."""
        ...

    async def read(self) -> T:
        """Fetch the entire contents of the stream until EOF."""
        ...

    def _get_logs(self, skip_empty_messages: bool = True) -> collections.abc.AsyncGenerator[bytes, None]:
        """Streams sandbox logs from the server to the reader.

        Logs returned by this method may contain partial or multiple lines at a time.

        When the stream receives an EOF, it yields None. Once an EOF is received,
        subsequent invocations will not yield logs.
        """
        ...

    def _get_logs_by_line(self) -> collections.abc.AsyncGenerator[bytes, None]:
        """Process logs from the server and yield complete lines only."""
        ...

    def __aiter__(self) -> collections.abc.AsyncGenerator[T, None]: ...
    async def aclose(self):
        """mdmd:hidden"""
        ...

def _decode_bytes_stream_to_str(
    stream: collections.abc.AsyncGenerator[bytes, None],
) -> collections.abc.AsyncGenerator[str, None]:
    """Incrementally decode a bytes async generator as UTF-8 without breaking on chunk boundaries.

    This function uses a streaming UTF-8 decoder so that multi-byte characters split across
    chunks are handled correctly instead of raising ``UnicodeDecodeError``.
    """
    ...

def _stream_by_line(stream: collections.abc.AsyncGenerator[bytes, None]) -> collections.abc.AsyncGenerator[bytes, None]:
    """Yield complete lines only (ending with
    ), buffering partial lines until complete.

        When this generator returns, the underlying generator is closed.

    """
    ...

class _StreamReaderThroughServerParams:
    """Parameters for a ``_StreamReader`` that reads sandbox logs through the server."""

    file_descriptor: int
    object_id: str
    client: modal.client._Client

    def __init__(self, file_descriptor: int, object_id: str, client: modal.client._Client) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

    def __setattr__(self, name, value):
        """Implement setattr(self, name, value)."""
        ...

    def __delattr__(self, name):
        """Implement delattr(self, name)."""
        ...

    def __hash__(self):
        """Return hash(self)."""
        ...

class _StreamReaderThroughSandboxExecCommandRouterParams:
    """Parameters for a ``_StreamReader`` that reads sandbox-exec stdio
    directly from the worker via the task command router (``exec_stdio_read``).
    """

    file_descriptor: int
    task_id: str
    object_id: str
    command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient
    deadline: typing.Optional[float]

    def __init__(
        self,
        file_descriptor: int,
        task_id: str,
        object_id: str,
        command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
        deadline: typing.Optional[float],
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

    def __setattr__(self, name, value):
        """Implement setattr(self, name, value)."""
        ...

    def __delattr__(self, name):
        """Implement delattr(self, name)."""
        ...

    def __hash__(self):
        """Return hash(self)."""
        ...

class _StreamReaderThroughSandboxCommandRouterParams:
    """Parameters for a ``_StreamReader`` that reads a V2 sandbox's
    stdio directly from the worker via the task command router
    (``sandbox_stdio_read``).
    """

    file_descriptor: int
    sandbox_id: str
    resolve_router: collections.abc.Callable[
        [], collections.abc.Awaitable[tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]]
    ]

    def __init__(
        self,
        file_descriptor: int,
        sandbox_id: str,
        resolve_router: collections.abc.Callable[
            [], collections.abc.Awaitable[tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]]
        ],
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

    def __setattr__(self, name, value):
        """Implement setattr(self, name, value)."""
        ...

    def __delattr__(self, name):
        """Implement delattr(self, name)."""
        ...

    def __hash__(self):
        """Return hash(self)."""
        ...

def _stdio_stream_from_sandbox_command_router(
    params: _StreamReaderThroughSandboxCommandRouterParams,
) -> collections.abc.AsyncGenerator[bytes, None]:
    """Stream raw bytes from a V2 sandbox's primary stdio via ``sandbox_stdio_read``."""
    ...

def _stdio_stream_from_sandbox_exec_command_router(
    params: _StreamReaderThroughSandboxExecCommandRouterParams,
) -> collections.abc.AsyncGenerator[bytes, None]:
    """Stream raw bytes from a V2 sandbox-exec'd process via ``exec_stdio_read``."""
    ...

def _stdio_stream_from_command_router(
    params: typing.Union[
        _StreamReaderThroughSandboxExecCommandRouterParams, _StreamReaderThroughSandboxCommandRouterParams
    ],
) -> collections.abc.AsyncGenerator[bytes, None]:
    """Dispatch between the V2-sandbox primary stdio and the V2-sandbox-exec
    stdio streams, both of which yield raw bytes.
    """
    ...

class _BytesStreamReaderThroughCommandRouter:
    """StreamReader that yields raw bytes from the router-backed stdio source
    (either V2 sandbox top-level stdio or V2 sandbox-exec stdio).
    """
    def __init__(
        self,
        params: typing.Union[
            _StreamReaderThroughSandboxExecCommandRouterParams, _StreamReaderThroughSandboxCommandRouterParams
        ],
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    @property
    def file_descriptor(self) -> int: ...
    async def read(self) -> bytes: ...
    def __aiter__(self) -> collections.abc.AsyncGenerator[bytes, None]: ...
    async def _print_all(self, output_stream: typing.TextIO) -> None: ...

class _TextStreamReaderThroughCommandRouter:
    """StreamReader that yields UTF-8-decoded text from the router-backed
    stdio source.
    """
    def __init__(
        self,
        params: typing.Union[
            _StreamReaderThroughSandboxExecCommandRouterParams, _StreamReaderThroughSandboxCommandRouterParams
        ],
        by_line: bool,
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    @property
    def file_descriptor(self) -> int: ...
    async def read(self) -> str: ...
    def __aiter__(self) -> collections.abc.AsyncGenerator[str, None]: ...
    async def _print_all(self, output_stream: typing.TextIO) -> None: ...

class _StdoutPrintingStreamReaderThroughCommandRouter(typing.Generic[T]):
    """StreamReader implementation for StreamType.STDOUT when using the task command router.

    This mirrors the behavior from the server-backed implementation: the stream is printed to
    the local stdout immediately and is not readable via StreamReader methods.
    """

    _reader: typing.Union[_TextStreamReaderThroughCommandRouter, _BytesStreamReaderThroughCommandRouter]

    def __init__(
        self, reader: typing.Union[_TextStreamReaderThroughCommandRouter, _BytesStreamReaderThroughCommandRouter]
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    @property
    def file_descriptor(self) -> int: ...
    def _start_printing_task(self) -> None: ...
    async def read(self) -> T: ...
    def __aiter__(self) -> collections.abc.AsyncIterator[T]: ...
    async def __anext__(self) -> T: ...
    async def aclose(self): ...

class _DevnullStreamReader(typing.Generic[T]):
    """StreamReader implementation for a stream configured with
    StreamType.DEVNULL. Throws an error if read or any other method is
    called.
    """
    def __init__(self, file_descriptor: int) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    @property
    def file_descriptor(self) -> int: ...
    async def read(self) -> T: ...
    def __aiter__(self) -> collections.abc.AsyncIterator[T]: ...
    async def __anext__(self) -> T: ...
    async def aclose(self): ...

class _StreamReader(typing.Generic[T]):
    """Retrieve logs from a stream (`stdout` or `stderr`).

    As an asynchronous iterable, the object supports the `for` and `async for`
    statements. Just loop over the object to read in chunks.
    """

    _impl: typing.Union[
        _StreamReaderThroughServer,
        _DevnullStreamReader,
        _TextStreamReaderThroughCommandRouter,
        _BytesStreamReaderThroughCommandRouter,
        _StdoutPrintingStreamReaderThroughCommandRouter,
    ]
    _read_gen: typing.Optional[collections.abc.AsyncGenerator[T, None]]

    def __init__(
        self,
        params: typing.Union[
            _StreamReaderThroughServerParams,
            _StreamReaderThroughSandboxExecCommandRouterParams,
            _StreamReaderThroughSandboxCommandRouterParams,
        ],
        *,
        stream_type: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        text: bool = True,
        by_line: bool = False,
    ) -> None:
        """mdmd:hidden"""
        ...

    @property
    def file_descriptor(self) -> int:
        """Possible values are `1` for stdout and `2` for stderr."""
        ...

    async def read(self) -> T:
        """Fetch the entire contents of the stream until EOF."""
        ...

    def __aiter__(self) -> collections.abc.AsyncGenerator[T, None]: ...
    async def __anext__(self) -> T:
        """Deprecated: This exists for backwards compatibility and will be removed in a future version of Modal

        Only use next/anext on the return value of iter/aiter on the StreamReader object (treat streamreader as
        an iterable, not an iterator).
        """
        ...

    async def aclose(self):
        """mdmd:hidden"""
        ...

class _StreamWriterThroughServerParams:
    """Parameters for a ``_StreamWriter`` that writes sandbox stdin through the server."""

    object_id: str
    client: modal.client._Client

    def __init__(self, object_id: str, client: modal.client._Client) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

    def __setattr__(self, name, value):
        """Implement setattr(self, name, value)."""
        ...

    def __delattr__(self, name):
        """Implement delattr(self, name)."""
        ...

    def __hash__(self):
        """Return hash(self)."""
        ...

class _StreamWriterThroughCommandRouterSandboxExecParams:
    """Parameters for a ``_StreamWriter`` that writes the stdin of a process
    spawned via ``sb.exec(...)`` directly to the worker via the task command
    router.
    """

    task_id: str
    object_id: str
    command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient

    def __init__(
        self,
        task_id: str,
        object_id: str,
        command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

    def __setattr__(self, name, value):
        """Implement setattr(self, name, value)."""
        ...

    def __delattr__(self, name):
        """Implement delattr(self, name)."""
        ...

    def __hash__(self):
        """Return hash(self)."""
        ...

class _StreamWriterThroughCommandRouterSandboxParams:
    """Parameters for a ``_StreamWriter`` that writes a V2 sandbox entrypoint's
    stdin directly to the worker via the task command router.
    """

    resolve_router: collections.abc.Callable[
        [], collections.abc.Awaitable[tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]]
    ]

    def __init__(
        self,
        resolve_router: collections.abc.Callable[
            [], collections.abc.Awaitable[tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]]
        ],
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

    def __setattr__(self, name, value):
        """Implement setattr(self, name, value)."""
        ...

    def __delattr__(self, name):
        """Implement delattr(self, name)."""
        ...

    def __hash__(self):
        """Return hash(self)."""
        ...

class _StreamWriterThroughServer:
    """Provides an interface to buffer and write to a sandbox stream (`stdin`) via the server."""
    def __init__(self, params: _StreamWriterThroughServerParams) -> None:
        """mdmd:hidden"""
        ...

    def _get_next_index(self) -> int: ...
    def write(self, data: typing.Union[bytes, bytearray, memoryview, str]) -> None:
        """Write data to the stream but does not send it immediately.

        This is non-blocking and queues the data to an internal buffer. Must be
        used along with the `drain()` method, which flushes the buffer.
        """
        ...

    def write_eof(self) -> None:
        """Close the write end of the stream after the buffered data is drained.

        If the process was blocked on input, it will become unblocked after
        `write_eof()`. This method needs to be used along with the `drain()`
        method, which flushes the EOF to the process.
        """
        ...

    async def drain(self) -> None:
        """Flush the write buffer and send data to the running process.

        This is a flow control method that blocks until data is sent. It returns
        when it is appropriate to continue writing data to the stream.
        """
        ...

class _StreamWriterThroughCommandRouterBuffer(abc.ABC):
    """Buffering/draining logic for stream writers that flush data
    to the task command router.
    """
    def __init__(self) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    async def stdin_write(self, data: bytes, eof: bool) -> None:
        """Write the given chunk (with optional EOF) to the command router."""
        ...

    def write(self, data: typing.Union[bytes, bytearray, memoryview, str]) -> None: ...
    def write_eof(self) -> None: ...
    async def drain(self) -> None: ...

class _StreamWriterThroughCommandRouterSandboxExec(_StreamWriterThroughCommandRouterBuffer):
    """Buffering/draining logic for stream writers that flush data
    to the task command router.
    """
    def __init__(self, params: _StreamWriterThroughCommandRouterSandboxExecParams) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    async def stdin_write(self, data: bytes, eof: bool) -> None:
        """Write the given chunk (with optional EOF) to the command router."""
        ...

class _StreamWriterThroughCommandRouterSandbox(_StreamWriterThroughCommandRouterBuffer):
    """Write a V2 sandbox entrypoint's stdin directly to the worker
    via the task command router.
    """
    def __init__(self, params: _StreamWriterThroughCommandRouterSandboxParams) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    async def stdin_write(self, data: bytes, eof: bool) -> None:
        """Write the given chunk (with optional EOF) to the command router."""
        ...

class _StreamWriter:
    """Provides an interface to buffer and write logs to a sandbox or container process stream (`stdin`)."""
    def __init__(
        self,
        params: typing.Union[
            _StreamWriterThroughServerParams,
            _StreamWriterThroughCommandRouterSandboxExecParams,
            _StreamWriterThroughCommandRouterSandboxParams,
        ],
    ) -> None:
        """mdmd:hidden"""
        ...

    def write(self, data: typing.Union[bytes, bytearray, memoryview, str]) -> None:
        """Write data to the stream but does not send it immediately.

        This is non-blocking and queues the data to an internal buffer. Must be
        used along with the `drain()` method, which flushes the buffer.

        Examples:
            ```python fixture:sandbox
            proc = sandbox.exec(
                "bash",
                "-c",
                "while read line; do echo $line; done",
            )
            proc.stdin.write(b"foo\n")
            proc.stdin.write(b"bar\n")
            proc.stdin.write_eof()
            proc.stdin.drain()
            ```
        """
        ...

    def write_eof(self) -> None:
        """Close the write end of the stream after the buffered data is drained.

        If the process was blocked on input, it will become unblocked after
        `write_eof()`. This method needs to be used along with the `drain()`
        method, which flushes the EOF to the process.
        """
        ...

    async def drain(self) -> None:
        """Flush the write buffer and send data to the running process.

        This is a flow control method that blocks until data is sent. It returns
        when it is appropriate to continue writing data to the stream.

        Examples:
            ```python notest
            writer.write(data)
            writer.drain()
            ```

            Async usage:

            ```python notest
            writer.write(data)  # not a blocking operation
            await writer.drain.aio()
            ```
        """
        ...

T_INNER = typing.TypeVar("T_INNER", covariant=True)

class StreamReader(typing.Generic[T]):
    """Retrieve logs from a stream (`stdout` or `stderr`).

    As an asynchronous iterable, the object supports the `for` and `async for`
    statements. Just loop over the object to read in chunks.
    """

    _impl: typing.Union[
        _StreamReaderThroughServer,
        _DevnullStreamReader,
        _TextStreamReaderThroughCommandRouter,
        _BytesStreamReaderThroughCommandRouter,
        _StdoutPrintingStreamReaderThroughCommandRouter,
    ]
    _read_gen: typing.Optional[collections.abc.AsyncGenerator[T, None]]

    def __init__(
        self,
        params: typing.Union[
            _StreamReaderThroughServerParams,
            _StreamReaderThroughSandboxExecCommandRouterParams,
            _StreamReaderThroughSandboxCommandRouterParams,
        ],
        *,
        stream_type: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        text: bool = True,
        by_line: bool = False,
    ) -> None:
        """mdmd:hidden"""
        ...

    @property
    def file_descriptor(self) -> int:
        """Possible values are `1` for stdout and `2` for stderr."""
        ...

    class __read_spec(typing_extensions.Protocol[T_INNER]):
        def __call__(self, /) -> T_INNER:
            """Fetch the entire contents of the stream until EOF."""
            ...

        async def aio(self, /) -> T_INNER:
            """Fetch the entire contents of the stream until EOF."""
            ...

    read: __read_spec[T]

    def __iter__(self) -> typing.Generator[T, None, None]: ...
    def __aiter__(self) -> collections.abc.AsyncGenerator[T, None]: ...
    def __next__(self) -> T:
        """Deprecated: This exists for backwards compatibility and will be removed in a future version of Modal

        Only use next/anext on the return value of iter/aiter on the StreamReader object (treat streamreader as
        an iterable, not an iterator).
        """
        ...

    async def __anext__(self) -> T:
        """Deprecated: This exists for backwards compatibility and will be removed in a future version of Modal

        Only use next/anext on the return value of iter/aiter on the StreamReader object (treat streamreader as
        an iterable, not an iterator).
        """
        ...

    def close(self):
        """mdmd:hidden"""
        ...

    async def aclose(self):
        """mdmd:hidden"""
        ...

class StreamWriter:
    """Provides an interface to buffer and write logs to a sandbox or container process stream (`stdin`)."""
    def __init__(
        self,
        params: typing.Union[
            _StreamWriterThroughServerParams,
            _StreamWriterThroughCommandRouterSandboxExecParams,
            _StreamWriterThroughCommandRouterSandboxParams,
        ],
    ) -> None:
        """mdmd:hidden"""
        ...

    def write(self, data: typing.Union[bytes, bytearray, memoryview, str]) -> None:
        """Write data to the stream but does not send it immediately.

        This is non-blocking and queues the data to an internal buffer. Must be
        used along with the `drain()` method, which flushes the buffer.

        Examples:
            ```python fixture:sandbox
            proc = sandbox.exec(
                "bash",
                "-c",
                "while read line; do echo $line; done",
            )
            proc.stdin.write(b"foo\n")
            proc.stdin.write(b"bar\n")
            proc.stdin.write_eof()
            proc.stdin.drain()
            ```
        """
        ...

    def write_eof(self) -> None:
        """Close the write end of the stream after the buffered data is drained.

        If the process was blocked on input, it will become unblocked after
        `write_eof()`. This method needs to be used along with the `drain()`
        method, which flushes the EOF to the process.
        """
        ...

    class __drain_spec(typing_extensions.Protocol):
        def __call__(self, /) -> None:
            """Flush the write buffer and send data to the running process.

            This is a flow control method that blocks until data is sent. It returns
            when it is appropriate to continue writing data to the stream.

            Examples:
                ```python notest
                writer.write(data)
                writer.drain()
                ```

                Async usage:

                ```python notest
                writer.write(data)  # not a blocking operation
                await writer.drain.aio()
                ```
            """
            ...

        async def aio(self, /) -> None:
            """Flush the write buffer and send data to the running process.

            This is a flow control method that blocks until data is sent. It returns
            when it is appropriate to continue writing data to the stream.

            Examples:
                ```python notest
                writer.write(data)
                writer.drain()
                ```

                Async usage:

                ```python notest
                writer.write(data)  # not a blocking operation
                await writer.drain.aio()
                ```
            """
            ...

    drain: __drain_spec

TASK_COMMAND_ROUTER_MAX_BUFFER_SIZE: int
