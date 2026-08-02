import modal._runtime.gpu_memory_snapshot
import modal.client
import modal_proto.api_pb2
import synchronicity.combined_types
import typing
import typing_extensions

class UserException(Exception):
    """Used to shut down the task gracefully."""

    ...

class _TaskLifecycleManager:
    _singleton: typing.ClassVar[typing.Optional[_TaskLifecycleManager]]
    task_id: str
    function_id: str
    function_def: modal_proto.api_pb2.Function
    checkpoint_id: typing.Optional[str]
    _client: modal.client._Client
    _cuda_checkpoint_session: typing.Optional[modal._runtime.gpu_memory_snapshot.CudaCheckpointSession]

    def _init(
        self,
        task_id: str,
        function_id: str,
        function_def: modal_proto.api_pb2.Function,
        checkpoint_id: typing.Optional[str],
        client: modal.client._Client,
    ) -> None: ...
    @staticmethod
    def __new__(
        cls,
        task_id: str,
        function_id: str,
        function_def: modal_proto.api_pb2.Function,
        checkpoint_id: typing.Optional[str],
        client: modal.client._Client,
    ) -> _TaskLifecycleManager:
        """Create and return a new object.  See help(type) for accurate signature."""
        ...

    @classmethod
    def _reset_singleton(cls) -> None:
        """Only used for tests."""
        ...

    def handle_task_lifecycle_exception(self) -> typing.AsyncContextManager[None]:
        """Report lifecycle exceptions as task-level failures and stop the task."""
        ...

    async def volume_commit(self, volume_ids: list[str]) -> None:
        """Perform volume commit for given `volume_ids`.
        Only used on container exit to persist uncommitted changes on behalf of user.
        """
        ...

    async def memory_restore(self) -> None: ...
    async def memory_snapshot(self) -> None:
        """Message server indicating that function is ready to be checkpointed."""
        ...

class TaskLifecycleManager:
    _singleton: typing.ClassVar[typing.Optional[TaskLifecycleManager]]
    task_id: str
    function_id: str
    function_def: modal_proto.api_pb2.Function
    checkpoint_id: typing.Optional[str]
    _client: modal.client.Client
    _cuda_checkpoint_session: typing.Optional[modal._runtime.gpu_memory_snapshot.CudaCheckpointSession]

    def __init__(self, /, *args, **kwargs):
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def _init(
        self,
        task_id: str,
        function_id: str,
        function_def: modal_proto.api_pb2.Function,
        checkpoint_id: typing.Optional[str],
        client: modal.client.Client,
    ) -> None: ...
    @classmethod
    def _reset_singleton(cls) -> None:
        """Only used for tests."""
        ...

    class __handle_task_lifecycle_exception_spec(typing_extensions.Protocol):
        def __call__(self, /) -> synchronicity.combined_types.AsyncAndBlockingContextManager[None]:
            """Report lifecycle exceptions as task-level failures and stop the task."""
            ...

        def aio(self, /) -> typing.AsyncContextManager[None]:
            """Report lifecycle exceptions as task-level failures and stop the task."""
            ...

    handle_task_lifecycle_exception: __handle_task_lifecycle_exception_spec

    class __volume_commit_spec(typing_extensions.Protocol):
        def __call__(self, /, volume_ids: list[str]) -> None:
            """Perform volume commit for given `volume_ids`.
            Only used on container exit to persist uncommitted changes on behalf of user.
            """
            ...

        async def aio(self, /, volume_ids: list[str]) -> None:
            """Perform volume commit for given `volume_ids`.
            Only used on container exit to persist uncommitted changes on behalf of user.
            """
            ...

    volume_commit: __volume_commit_spec

    class __memory_restore_spec(typing_extensions.Protocol):
        def __call__(self, /) -> None: ...
        async def aio(self, /) -> None: ...

    memory_restore: __memory_restore_spec

    class __memory_snapshot_spec(typing_extensions.Protocol):
        def __call__(self, /) -> None:
            """Message server indicating that function is ready to be checkpointed."""
            ...

        async def aio(self, /) -> None:
            """Message server indicating that function is ready to be checkpointed."""
            ...

    memory_snapshot: __memory_snapshot_spec

def check_fastapi_pydantic_compatibility(exc: ImportError) -> None:
    """Add a helpful note to an exception that is likely caused by a pydantic<>fastapi version incompatibility.

    We need this becasue the legacy set of container requirements (image_builder_version=2023.12) contains a
    version of fastapi that is not forwards-compatible with pydantic 2.0+, and users commonly run into issues
    building an image that specifies a more recent version only for pydantic.
    """
    ...
