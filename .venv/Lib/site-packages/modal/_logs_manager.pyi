import asyncio
import collections.abc
import datetime
import enum
import modal._supports_logs
import modal.types
import modal_proto.api_pb2
import typing
import typing_extensions

LogSource = typing.Literal["stdout", "stderr", "system"]

class _Deadline:
    """_Deadline(value: 'float | None' = None)"""

    value: typing.Optional[float]

    def reset(self, timeout: typing.Optional[float] = None) -> None: ...
    def __init__(self, value: typing.Optional[float] = None) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

class _StreamStopReason(enum.Enum):
    IDLE_TIMEOUT = "idle_timeout"
    STOP_STREAM = "stop_stream"

class _LogsManager:
    """mdmd:namespace"""
    def __init__(
        self,
        source: modal._supports_logs._SupportsLogs,
        stop_stream: typing.Optional[collections.abc.Callable[[], collections.abc.Awaitable[bool]]] = None,
    ):
        """mdmd:hidden"""
        ...

    async def _params(self) -> modal._supports_logs._LogQueryData: ...
    @staticmethod
    def _resolve_source(source: typing.Optional[typing.Literal["stdout", "stderr", "system"]]) -> int: ...
    @staticmethod
    def _normalize_utc_datetime(value: datetime.datetime, name: str) -> datetime.datetime: ...
    @staticmethod
    def _entry_source(file_descriptor: int) -> typing.Literal["stdout", "stderr", "system"]: ...
    @staticmethod
    def _entry_timestamp(item: modal_proto.api_pb2.TaskLogs) -> datetime.datetime: ...
    def _entry_context_ids(
        self, item: modal_proto.api_pb2.TaskLogs, batch: modal_proto.api_pb2.TaskLogsBatch
    ) -> list[str]: ...
    def _entry_from_item(
        self, item: modal_proto.api_pb2.TaskLogs, batch: modal_proto.api_pb2.TaskLogsBatch
    ) -> modal.types.LogEntry: ...
    async def _watch_stream_stop(self, deadline: _Deadline) -> typing.Optional[_StreamStopReason]: ...
    def fetch(
        self,
        *,
        since: datetime.datetime,
        until: typing.Optional[datetime.datetime] = None,
        source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
        search_text: str = "",
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch all associated logs corresponding to the date range and filters."""
        ...

    def tail(
        self, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch the most recent logs."""
        ...

    def _create_log_stream(
        self, params: modal._supports_logs._LogQueryData, last_entry_id: str, timeout: float
    ) -> collections.abc.AsyncGenerator[modal_proto.api_pb2.TaskLogsBatch, None]:
        """Helper for creating the log stream RPC."""
        ...

    def _stream_entries(self, batch: modal_proto.api_pb2.TaskLogsBatch): ...
    @staticmethod
    def _advance_batch(batch: modal_proto.api_pb2.TaskLogsBatch, last_entry_id: str) -> tuple[str, bool]: ...
    @staticmethod
    def _is_transient_stream_error(exc: Exception) -> bool: ...
    def _drain_stream(
        self, params: modal._supports_logs._LogQueryData, last_entry_id: str, timeout: float
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Do a final bounded drain in the logs to catch any remaining logs after the stop condition is met."""
        ...

    def stream(
        self, timeout: typing.Optional[float] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Stream new logs until no logs arrive within the timeout."""
        ...

    @staticmethod
    async def _suppress_cancelled(task: asyncio.Task) -> None: ...

class LogsManager:
    """mdmd:namespace"""
    def __init__(
        self,
        source: modal._supports_logs._SupportsLogs,
        stop_stream: typing.Optional[collections.abc.Callable[[], bool]] = None,
    ):
        """mdmd:hidden"""
        ...

    class ___params_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal._supports_logs._LogQueryData: ...
        async def aio(self, /) -> modal._supports_logs._LogQueryData: ...

    _params: ___params_spec

    @staticmethod
    def _resolve_source(source: typing.Optional[typing.Literal["stdout", "stderr", "system"]]) -> int: ...
    @staticmethod
    def _normalize_utc_datetime(value: datetime.datetime, name: str) -> datetime.datetime: ...
    @staticmethod
    def _entry_source(file_descriptor: int) -> typing.Literal["stdout", "stderr", "system"]: ...
    @staticmethod
    def _entry_timestamp(item: modal_proto.api_pb2.TaskLogs) -> datetime.datetime: ...
    def _entry_context_ids(
        self, item: modal_proto.api_pb2.TaskLogs, batch: modal_proto.api_pb2.TaskLogsBatch
    ) -> list[str]: ...
    def _entry_from_item(
        self, item: modal_proto.api_pb2.TaskLogs, batch: modal_proto.api_pb2.TaskLogsBatch
    ) -> modal.types.LogEntry: ...

    class ___watch_stream_stop_spec(typing_extensions.Protocol):
        def __call__(self, /, deadline: _Deadline) -> typing.Optional[_StreamStopReason]: ...
        async def aio(self, /, deadline: _Deadline) -> typing.Optional[_StreamStopReason]: ...

    _watch_stream_stop: ___watch_stream_stop_spec

    class __fetch_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            since: datetime.datetime,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch all associated logs corresponding to the date range and filters."""
            ...

        def aio(
            self,
            /,
            *,
            since: datetime.datetime,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch all associated logs corresponding to the date range and filters."""
            ...

    fetch: __fetch_spec

    class __tail_spec(typing_extensions.Protocol):
        def __call__(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch the most recent logs."""
            ...

        def aio(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch the most recent logs."""
            ...

    tail: __tail_spec

    class ___create_log_stream_spec(typing_extensions.Protocol):
        def __call__(
            self, /, params: modal._supports_logs._LogQueryData, last_entry_id: str, timeout: float
        ) -> typing.Generator[modal_proto.api_pb2.TaskLogsBatch, None, None]:
            """Helper for creating the log stream RPC."""
            ...

        def aio(
            self, /, params: modal._supports_logs._LogQueryData, last_entry_id: str, timeout: float
        ) -> collections.abc.AsyncGenerator[modal_proto.api_pb2.TaskLogsBatch, None]:
            """Helper for creating the log stream RPC."""
            ...

    _create_log_stream: ___create_log_stream_spec

    def _stream_entries(self, batch: modal_proto.api_pb2.TaskLogsBatch): ...
    @staticmethod
    def _advance_batch(batch: modal_proto.api_pb2.TaskLogsBatch, last_entry_id: str) -> tuple[str, bool]: ...
    @staticmethod
    def _is_transient_stream_error(exc: Exception) -> bool: ...

    class ___drain_stream_spec(typing_extensions.Protocol):
        def __call__(
            self, /, params: modal._supports_logs._LogQueryData, last_entry_id: str, timeout: float
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Do a final bounded drain in the logs to catch any remaining logs after the stop condition is met."""
            ...

        def aio(
            self, /, params: modal._supports_logs._LogQueryData, last_entry_id: str, timeout: float
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Do a final bounded drain in the logs to catch any remaining logs after the stop condition is met."""
            ...

    _drain_stream: ___drain_stream_spec

    class __stream_spec(typing_extensions.Protocol):
        def __call__(
            self, /, timeout: typing.Optional[float] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Stream new logs until no logs arrive within the timeout."""
            ...

        def aio(
            self, /, timeout: typing.Optional[float] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Stream new logs until no logs arrive within the timeout."""
            ...

    stream: __stream_spec

    class ___suppress_cancelled_spec(typing_extensions.Protocol):
        def __call__(self, /, task: asyncio.Task) -> None: ...
        async def aio(self, /, task: asyncio.Task) -> None: ...

    _suppress_cancelled: typing.ClassVar[___suppress_cancelled_spec]

class _FunctionLogsManager(_LogsManager):
    """mdmd:namespace"""
    def __init__(self, source: modal._supports_logs._SupportsLogs):
        """mdmd:hidden"""
        ...

    def fetch(
        self,
        *,
        since: datetime.datetime,
        until: typing.Optional[datetime.datetime] = None,
        source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
        search_text: str = "",
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch Function logs corresponding to the date range and filters.

        Args:
            since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
            until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                as local time.
            source: Filter by source: 'stdout', 'stderr', or 'system'.
            search_text: Filter by search text.

        Yields:
            `LogEntry` objects in chronological order.

        Examples:

            ```python notest
            function = modal.Function.from_name("my-app", "train")

            for entry in function.logs.fetch(
                since=datetime.now() - timedelta(hours=4),
                source="stdout",
            ):
                print(entry.message, end="")
            ```
        """
        ...

    def tail(
        self, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch the most recent Function logs.

        Args:
            entries: The number of log entries to return.
            source: Filter by source: 'stdout', 'stderr', or 'system'.

        Yields:
            `LogEntry` objects in chronological order.

        Examples:

            ```python notest
            function = modal.Function.from_name("my-app", "train")

            for entry in function.logs.tail(20):
                print(entry.message, end="")
            ```
        """
        ...

    def stream(
        self, timeout: typing.Optional[float] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Stream new Function logs until the timeout is reached.

        Args:
            timeout: Number of seconds to wait between log entries before terminating the stream.
                By default, this will block until it is interrupted.

        Yields:
            `LogEntry` objects as they arrive.

        Examples:

            ```python notest
            function = modal.Function.from_name("my-app", "train")

            for entry in function.logs.stream(timeout=60):
                print(entry.message, end="")
            ```
        """
        ...

class FunctionLogsManager(LogsManager):
    """mdmd:namespace"""
    def __init__(self, source: modal._supports_logs._SupportsLogs):
        """mdmd:hidden"""
        ...

    class __fetch_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            since: datetime.datetime,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch Function logs corresponding to the date range and filters.

            Args:
                since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                    as local time.
                source: Filter by source: 'stdout', 'stderr', or 'system'.
                search_text: Filter by search text.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")

                for entry in function.logs.fetch(
                    since=datetime.now() - timedelta(hours=4),
                    source="stdout",
                ):
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self,
            /,
            *,
            since: datetime.datetime,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch Function logs corresponding to the date range and filters.

            Args:
                since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                    as local time.
                source: Filter by source: 'stdout', 'stderr', or 'system'.
                search_text: Filter by search text.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")

                for entry in function.logs.fetch(
                    since=datetime.now() - timedelta(hours=4),
                    source="stdout",
                ):
                    print(entry.message, end="")
                ```
            """
            ...

    fetch: __fetch_spec

    class __tail_spec(typing_extensions.Protocol):
        def __call__(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch the most recent Function logs.

            Args:
                entries: The number of log entries to return.
                source: Filter by source: 'stdout', 'stderr', or 'system'.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")

                for entry in function.logs.tail(20):
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch the most recent Function logs.

            Args:
                entries: The number of log entries to return.
                source: Filter by source: 'stdout', 'stderr', or 'system'.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")

                for entry in function.logs.tail(20):
                    print(entry.message, end="")
                ```
            """
            ...

    tail: __tail_spec

    class __stream_spec(typing_extensions.Protocol):
        def __call__(
            self, /, timeout: typing.Optional[float] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Stream new Function logs until the timeout is reached.

            Args:
                timeout: Number of seconds to wait between log entries before terminating the stream.
                    By default, this will block until it is interrupted.

            Yields:
                `LogEntry` objects as they arrive.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")

                for entry in function.logs.stream(timeout=60):
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self, /, timeout: typing.Optional[float] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Stream new Function logs until the timeout is reached.

            Args:
                timeout: Number of seconds to wait between log entries before terminating the stream.
                    By default, this will block until it is interrupted.

            Yields:
                `LogEntry` objects as they arrive.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")

                for entry in function.logs.stream(timeout=60):
                    print(entry.message, end="")
                ```
            """
            ...

    stream: __stream_spec

class _ServerLogsManager(_LogsManager):
    """mdmd:namespace"""
    def __init__(self, source: modal._supports_logs._SupportsLogs):
        """mdmd:hidden"""
        ...

    def fetch(
        self,
        *,
        since: datetime.datetime,
        until: typing.Optional[datetime.datetime] = None,
        source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
        search_text: str = "",
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch Server logs corresponding to the date range and filters.

        Args:
            since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
            until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                as local time.
            source: Filter by source: 'stdout', 'stderr', or 'system'.
            search_text: Filter by search text.

        Yields:
            `LogEntry` objects in chronological order.

        Examples:

            ```python notest
            server = modal.Server.from_name("my-app", "web")

            for entry in server.logs.fetch(
                since=datetime.now() - timedelta(minutes=25),
                source="stdout",
            ):
                print(entry.message, end="")
            ```
        """
        ...

    def tail(
        self, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch the most recent Server logs.

        Args:
            entries: The number of log entries to return.
            source: Filter by source: 'stdout', 'stderr', or 'system'.

        Yields:
            `LogEntry` objects in chronological order.

        Examples:

            ```python notest
            server = modal.Server.from_name("my-app", "web")

            for entry in server.logs.tail(20):
                print(entry.message, end="")
            ```
        """
        ...

    def stream(
        self, timeout: typing.Optional[float] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Stream new Server logs until the timeout is reached.

        Args:
            timeout: Number of seconds to wait between log entries before terminating the stream.
                By default, this will block until it is interrupted.

        Yields:
            `LogEntry` objects as they arrive.

        Examples:

            ```python notest
            server = modal.Server.from_name("my-app", "web")

            for entry in server.logs.stream(timeout=60):
                print(entry.message, end="")
            ```
        """
        ...

class ServerLogsManager(LogsManager):
    """mdmd:namespace"""
    def __init__(self, source: modal._supports_logs._SupportsLogs):
        """mdmd:hidden"""
        ...

    class __fetch_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            since: datetime.datetime,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch Server logs corresponding to the date range and filters.

            Args:
                since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                    as local time.
                source: Filter by source: 'stdout', 'stderr', or 'system'.
                search_text: Filter by search text.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                server = modal.Server.from_name("my-app", "web")

                for entry in server.logs.fetch(
                    since=datetime.now() - timedelta(minutes=25),
                    source="stdout",
                ):
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self,
            /,
            *,
            since: datetime.datetime,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch Server logs corresponding to the date range and filters.

            Args:
                since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                    as local time.
                source: Filter by source: 'stdout', 'stderr', or 'system'.
                search_text: Filter by search text.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                server = modal.Server.from_name("my-app", "web")

                for entry in server.logs.fetch(
                    since=datetime.now() - timedelta(minutes=25),
                    source="stdout",
                ):
                    print(entry.message, end="")
                ```
            """
            ...

    fetch: __fetch_spec

    class __tail_spec(typing_extensions.Protocol):
        def __call__(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch the most recent Server logs.

            Args:
                entries: The number of log entries to return.
                source: Filter by source: 'stdout', 'stderr', or 'system'.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                server = modal.Server.from_name("my-app", "web")

                for entry in server.logs.tail(20):
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch the most recent Server logs.

            Args:
                entries: The number of log entries to return.
                source: Filter by source: 'stdout', 'stderr', or 'system'.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                server = modal.Server.from_name("my-app", "web")

                for entry in server.logs.tail(20):
                    print(entry.message, end="")
                ```
            """
            ...

    tail: __tail_spec

    class __stream_spec(typing_extensions.Protocol):
        def __call__(
            self, /, timeout: typing.Optional[float] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Stream new Server logs until the timeout is reached.

            Args:
                timeout: Number of seconds to wait between log entries before terminating the stream.
                    By default, this will block until it is interrupted.

            Yields:
                `LogEntry` objects as they arrive.

            Examples:

                ```python notest
                server = modal.Server.from_name("my-app", "web")

                for entry in server.logs.stream(timeout=60):
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self, /, timeout: typing.Optional[float] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Stream new Server logs until the timeout is reached.

            Args:
                timeout: Number of seconds to wait between log entries before terminating the stream.
                    By default, this will block until it is interrupted.

            Yields:
                `LogEntry` objects as they arrive.

            Examples:

                ```python notest
                server = modal.Server.from_name("my-app", "web")

                for entry in server.logs.stream(timeout=60):
                    print(entry.message, end="")
                ```
            """
            ...

    stream: __stream_spec

class _FunctionCallLogsManager(_LogsManager):
    """mdmd:namespace"""
    def __init__(self, source: modal._supports_logs._SupportsLogs):
        """mdmd:hidden"""
        ...

    async def _params(self) -> modal._supports_logs._LogQueryData: ...
    async def _get_function_call_info(self) -> modal_proto.api_pb2.FunctionCallInfo: ...
    async def _determine_function_call_stop(self) -> bool: ...
    def stream(
        self, timeout: typing.Optional[float] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Stream new FunctionCall logs until the timeout is reached.
        The timeout specifies the number of seconds to wait between log entries before terminating the stream.
        This method will stop when the FunctionCall is observed to have completed,
        or when the timeout is reached. The completion check is best-effort; if completion
        cannot be determined, the stream will continue until the timeout is reached.

        Args:
            timeout: Number of seconds to wait between log entries before terminating the stream.
               By default, this will block until it is interrupted.

        Yields:
            `LogEntry` objects as they arrive.

        Examples:

            ```python notest
            function = modal.Function.from_name("my-app", "train")
            call = function.spawn()

            for entry in call.logs.stream():
                print(entry.message, end="")
            ```
        """
        ...

    def tail(
        self, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch the most recent FunctionCall logs.

        Args:
            entries: The number of log entries to return.
            source: Filter by source: 'stdout', 'stderr', or 'system'.

        Yields:
            `LogEntry` objects in chronological order.

        Examples:

            ```python notest
            function = modal.Function.from_name("my-app", "train")
            call = function.spawn()

            for entry in call.logs.tail(entries=10):
                print(entry.timestamp, entry.message, end="")
            ```
        """
        ...

    def fetch(
        self,
        *,
        since: typing.Optional[datetime.datetime] = None,
        until: typing.Optional[datetime.datetime] = None,
        source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
        search_text: str = "",
    ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
        """Fetch all associated logs corresponding to the date range and filters.

        Args:
            since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                By default, this will fetch logs from the start of the function call.
            until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                as local time.
            source: Filter by source: 'stdout', 'stderr', or 'system'.
            search_text: Filter by search text.

        Yields:
            `LogEntry` objects in chronological order.

        Examples:

            ```python notest
            function = modal.Function.from_name("my-app", "train")
            call = function.spawn()

            for entry in call.logs.fetch():
                print(entry.timestamp, entry.message, end="")
            ```
        """
        ...

class FunctionCallLogsManager(LogsManager):
    """mdmd:namespace"""
    def __init__(self, source: modal._supports_logs._SupportsLogs):
        """mdmd:hidden"""
        ...

    class ___params_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal._supports_logs._LogQueryData: ...
        async def aio(self, /) -> modal._supports_logs._LogQueryData: ...

    _params: ___params_spec

    class ___get_function_call_info_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal_proto.api_pb2.FunctionCallInfo: ...
        async def aio(self, /) -> modal_proto.api_pb2.FunctionCallInfo: ...

    _get_function_call_info: ___get_function_call_info_spec

    class ___determine_function_call_stop_spec(typing_extensions.Protocol):
        def __call__(self, /) -> bool: ...
        async def aio(self, /) -> bool: ...

    _determine_function_call_stop: ___determine_function_call_stop_spec

    class __stream_spec(typing_extensions.Protocol):
        def __call__(
            self, /, timeout: typing.Optional[float] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Stream new FunctionCall logs until the timeout is reached.
            The timeout specifies the number of seconds to wait between log entries before terminating the stream.
            This method will stop when the FunctionCall is observed to have completed,
            or when the timeout is reached. The completion check is best-effort; if completion
            cannot be determined, the stream will continue until the timeout is reached.

            Args:
                timeout: Number of seconds to wait between log entries before terminating the stream.
                   By default, this will block until it is interrupted.

            Yields:
                `LogEntry` objects as they arrive.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")
                call = function.spawn()

                for entry in call.logs.stream():
                    print(entry.message, end="")
                ```
            """
            ...

        def aio(
            self, /, timeout: typing.Optional[float] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Stream new FunctionCall logs until the timeout is reached.
            The timeout specifies the number of seconds to wait between log entries before terminating the stream.
            This method will stop when the FunctionCall is observed to have completed,
            or when the timeout is reached. The completion check is best-effort; if completion
            cannot be determined, the stream will continue until the timeout is reached.

            Args:
                timeout: Number of seconds to wait between log entries before terminating the stream.
                   By default, this will block until it is interrupted.

            Yields:
                `LogEntry` objects as they arrive.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")
                call = function.spawn()

                for entry in call.logs.stream():
                    print(entry.message, end="")
                ```
            """
            ...

    stream: __stream_spec

    class __tail_spec(typing_extensions.Protocol):
        def __call__(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch the most recent FunctionCall logs.

            Args:
                entries: The number of log entries to return.
                source: Filter by source: 'stdout', 'stderr', or 'system'.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")
                call = function.spawn()

                for entry in call.logs.tail(entries=10):
                    print(entry.timestamp, entry.message, end="")
                ```
            """
            ...

        def aio(
            self, /, entries: int = 100, *, source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch the most recent FunctionCall logs.

            Args:
                entries: The number of log entries to return.
                source: Filter by source: 'stdout', 'stderr', or 'system'.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")
                call = function.spawn()

                for entry in call.logs.tail(entries=10):
                    print(entry.timestamp, entry.message, end="")
                ```
            """
            ...

    tail: __tail_spec

    class __fetch_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            since: typing.Optional[datetime.datetime] = None,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> typing.Generator[modal.types.LogEntry, None, None]:
            """Fetch all associated logs corresponding to the date range and filters.

            Args:
                since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                    By default, this will fetch logs from the start of the function call.
                until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                    as local time.
                source: Filter by source: 'stdout', 'stderr', or 'system'.
                search_text: Filter by search text.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")
                call = function.spawn()

                for entry in call.logs.fetch():
                    print(entry.timestamp, entry.message, end="")
                ```
            """
            ...

        def aio(
            self,
            /,
            *,
            since: typing.Optional[datetime.datetime] = None,
            until: typing.Optional[datetime.datetime] = None,
            source: typing.Optional[typing.Literal["stdout", "stderr", "system"]] = None,
            search_text: str = "",
        ) -> collections.abc.AsyncGenerator[modal.types.LogEntry, None]:
            """Fetch all associated logs corresponding to the date range and filters.

            Args:
                since: Start date to fetch logs from. Must be in UTC or timezone-naive, which is interpreted as local time.
                    By default, this will fetch logs from the start of the function call.
                until: Defaults to current date if None. Must be in UTC or timezone-naive, which is interpreted
                    as local time.
                source: Filter by source: 'stdout', 'stderr', or 'system'.
                search_text: Filter by search text.

            Yields:
                `LogEntry` objects in chronological order.

            Examples:

                ```python notest
                function = modal.Function.from_name("my-app", "train")
                call = function.spawn()

                for entry in call.logs.fetch():
                    print(entry.timestamp, entry.message, end="")
                ```
            """
            ...

    fetch: __fetch_spec
