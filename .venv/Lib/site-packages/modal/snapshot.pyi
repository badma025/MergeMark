import google.protobuf.message
import modal._object
import modal.client
import modal.object
import modal_proto.api_pb2
import typing
import typing_extensions

SUPERSELF = typing.TypeVar("SUPERSELF", covariant=True)

class _SandboxSnapshot(modal._object._Object):
    """> Sandbox memory snapshots are in **early preview**.

    A `SandboxSnapshot` object lets you interact with a stored Sandbox snapshot that was created by calling
    `._experimental_snapshot()` on a Sandbox instance. This includes both the filesystem and memory state of
    the original Sandbox at the time the snapshot was taken.
    """

    _metadata: typing.Optional[modal_proto.api_pb2.SandboxSnapshotHandleMetadata]

    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self) -> typing.Optional[modal_proto.api_pb2.SandboxSnapshotHandleMetadata]: ...
    @property
    def _is_v2(self) -> typing.Optional[bool]:
        """Whether the snapshot came from a V2 sandbox."""
        ...

    class __from_id_spec(typing_extensions.Protocol[SUPERSELF]):
        def __call__(
            self, /, sandbox_snapshot_id: str, client: typing.Optional[modal.client.Client] = None
        ) -> SUPERSELF:
            """Construct a `SandboxSnapshot` for an existing snapshot ID.

            Args:
                sandbox_snapshot_id: Snapshot ID returned when the snapshot was created.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                A `SandboxSnapshot` handle (hydration validates the ID when used).
            """
            ...

        async def aio(self, /, sandbox_snapshot_id: str, client: typing.Optional[modal.client.Client] = None): ...

    from_id: typing.ClassVar[__from_id_spec[typing_extensions.Self]]

class SandboxSnapshot(modal.object.Object):
    """> Sandbox memory snapshots are in **early preview**.

    A `SandboxSnapshot` object lets you interact with a stored Sandbox snapshot that was created by calling
    `._experimental_snapshot()` on a Sandbox instance. This includes both the filesystem and memory state of
    the original Sandbox at the time the snapshot was taken.
    """

    _metadata: typing.Optional[modal_proto.api_pb2.SandboxSnapshotHandleMetadata]

    def __init__(self, *args, **kwargs):
        """mdmd:hidden"""
        ...

    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self) -> typing.Optional[modal_proto.api_pb2.SandboxSnapshotHandleMetadata]: ...
    @property
    def _is_v2(self) -> typing.Optional[bool]:
        """Whether the snapshot came from a V2 sandbox."""
        ...

    class __from_id_spec(typing_extensions.Protocol[SUPERSELF]):
        def __call__(
            self, /, sandbox_snapshot_id: str, client: typing.Optional[modal.client.Client] = None
        ) -> SUPERSELF:
            """Construct a `SandboxSnapshot` for an existing snapshot ID.

            Args:
                sandbox_snapshot_id: Snapshot ID returned when the snapshot was created.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                A `SandboxSnapshot` handle (hydration validates the ID when used).
            """
            ...

        async def aio(self, /, sandbox_snapshot_id: str, client: typing.Optional[modal.client.Client] = None): ...

    from_id: typing.ClassVar[__from_id_spec[typing_extensions.Self]]
