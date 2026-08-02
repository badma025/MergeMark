import modal._utils.task_command_router_client
import modal.client
import modal.io_streams
import modal.stream_type
import typing
import typing_extensions

T = typing.TypeVar("T")

def _iter_stream_as_bytes(stream: modal.io_streams._StreamReader[T]):
    """Yield raw bytes from a StreamReader regardless of text mode/backend."""
    ...

class _ContainerProcess(typing.Generic[T]):
    """Represents a running process in a container.

    Container processes communicate via direct communication with
    the Modal worker where the container is running.
    """
    def __init__(
        self,
        process_id: str,
        task_id: str,
        client: modal.client._Client,
        command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        exec_deadline: typing.Optional[float] = None,
        text: bool = True,
        by_line: bool = False,
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self) -> str:
        """Return repr(self)."""
        ...

    @property
    def stdout(self) -> modal.io_streams._StreamReader[T]:
        """StreamReader for the container process's stdout stream."""
        ...

    @property
    def stderr(self) -> modal.io_streams._StreamReader[T]:
        """StreamReader for the container process's stderr stream."""
        ...

    @property
    def stdin(self) -> modal.io_streams._StreamWriter:
        """StreamWriter for the container process's stdin stream."""
        ...

    @property
    def returncode(self) -> int: ...
    async def _stdin_write_stream(self, source: typing.BinaryIO) -> int:
        """mdmd:hidden
        Stream `source` into the process's stdin and close it (EOF) on success.

        Returns the total bytes streamed.
        """
        ...

    async def poll(self) -> typing.Optional[int]:
        """Check if the container process has finished running.

        Returns `None` if the process is still running, else returns the exit code.
        """
        ...

    async def wait(self) -> int:
        """Wait for the container process to finish running. Returns the exit code."""
        ...

    async def attach(self):
        """mdmd:hidden"""
        ...

class ContainerProcess(typing.Generic[T]):
    """Represents a running process in a container.

    Container processes communicate via direct communication with
    the Modal worker where the container is running.
    """
    def __init__(
        self,
        process_id: str,
        task_id: str,
        client: modal.client.Client,
        command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        exec_deadline: typing.Optional[float] = None,
        text: bool = True,
        by_line: bool = False,
    ) -> None: ...
    def __repr__(self) -> str: ...
    @property
    def stdout(self) -> modal.io_streams.StreamReader[T]:
        """StreamReader for the container process's stdout stream."""
        ...

    @property
    def stderr(self) -> modal.io_streams.StreamReader[T]:
        """StreamReader for the container process's stderr stream."""
        ...

    @property
    def stdin(self) -> modal.io_streams.StreamWriter:
        """StreamWriter for the container process's stdin stream."""
        ...

    @property
    def returncode(self) -> int: ...

    class ___stdin_write_stream_spec(typing_extensions.Protocol):
        def __call__(self, /, source: typing.BinaryIO) -> int:
            """mdmd:hidden
            Stream `source` into the process's stdin and close it (EOF) on success.

            Returns the total bytes streamed.
            """
            ...

        async def aio(self, /, source: typing.BinaryIO) -> int:
            """mdmd:hidden
            Stream `source` into the process's stdin and close it (EOF) on success.

            Returns the total bytes streamed.
            """
            ...

    _stdin_write_stream: ___stdin_write_stream_spec

    class __poll_spec(typing_extensions.Protocol):
        def __call__(self, /) -> typing.Optional[int]:
            """Check if the container process has finished running.

            Returns `None` if the process is still running, else returns the exit code.
            """
            ...

        async def aio(self, /) -> typing.Optional[int]:
            """Check if the container process has finished running.

            Returns `None` if the process is still running, else returns the exit code.
            """
            ...

    poll: __poll_spec

    class __wait_spec(typing_extensions.Protocol):
        def __call__(self, /) -> int:
            """Wait for the container process to finish running. Returns the exit code."""
            ...

        async def aio(self, /) -> int:
            """Wait for the container process to finish running. Returns the exit code."""
            ...

    wait: __wait_spec

    class __attach_spec(typing_extensions.Protocol):
        def __call__(self, /):
            """mdmd:hidden"""
            ...

        async def aio(self, /):
            """mdmd:hidden"""
            ...

    attach: __attach_spec
