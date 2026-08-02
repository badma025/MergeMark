import _io
import asyncio.locks
import collections.abc
import datetime
import google.protobuf.message
import modal._object
import modal._utils.blob_utils
import modal.client
import modal.object
import modal.types
import modal_proto.api_pb2
import pathlib
import synchronicity
import synchronicity.combined_types
import typing
import typing_extensions

async def _raise_on_block_response_error(response) -> None:
    """Raise a picklable Modal exception on error.

    We avoid `aiohttp.ClientResponse.raise_for_status()` because the resulting
    `ClientResponseError` is not picklable, which breaks result serialization
    when raised from within a Modal function.
    """
    ...

def _validate_volume_version(requested_version: int, actual_version: int, volume_name: str) -> None:
    """Validate that the returned volume version matches the requested version."""
    ...

def _experimental_options_to_string_map(experimental_options: dict[str, typing.Any]) -> dict[str, str]:
    """Coerce experimental option values to strings for the proto's string map."""
    ...

def _volume_create_options_to_proto(
    create_options: typing.Optional[modal.types.VolumeCreateOptions],
) -> modal_proto.api_pb2.VolumeCreateOptions:
    """Build a VolumeCreateOptions proto message from the public TypedDict."""
    ...

class _VolumeManager:
    """Namespace with methods for managing named Volume objects."""
    async def create(
        self,
        name: str,
        *,
        version: typing.Optional[int] = None,
        allow_existing: bool = False,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
    ) -> None:
        """Create a new named Volume in the workspace environment.

        This does not return a local handle; use `modal.Volume.from_name` to look up the Volume after creation.

        Added in v1.1.2.

        Args:
            name: Name for the new Volume.
            version: Optional VolumeFS backend version (1 or 2); experimental.
            allow_existing: If True, do nothing when a Volume with this name already exists.
            environment_name: Environment to create in; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.
            experimental_options: Experimental options to create Volume with.

        Examples:
            ```python notest
            modal.Volume.objects.create("my-volume")
            ```

            Volumes will be created in the active environment, or another one can be specified:

            ```python notest
            modal.Volume.objects.create("my-volume", environment_name="dev")
            ```

            By default, an error is raised if the Volume already exists; `allow_existing=True` makes that case a no-op:

            ```python notest
            modal.Volume.objects.create("my-volume", allow_existing=True)
            ```

            Note that this method does not return a local instance of the Volume. You can use
            `modal.Volume.from_name` to perform a lookup after creation.
        """
        ...

    async def list(
        self,
        *,
        max_objects: typing.Optional[int] = None,
        created_before: typing.Union[datetime.datetime, str, None] = None,
        environment_name: str = "",
        client: typing.Optional[modal.client._Client] = None,
    ) -> list[_Volume]:
        """List named Volumes in the workspace environment as hydrated handles.

        Results are ordered newest to oldest. By default, all matching Volumes are returned.

        Added in v1.1.2.

        Args:
            max_objects: Maximum number of Volumes to return.
            created_before: Only include Volumes created before this time (datetime or ISO date string).
            environment_name: Environment to list from; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.

        Returns:
            Hydrated `Volume` objects for each named Volume in the listing.

        Examples:
            ```python
            volumes = modal.Volume.objects.list()
            print([v.name for v in volumes])
            ```

            Volumes will be retrieved from the active environment, or another one can be specified:

            ```python notest
            dev_volumes = modal.Volume.objects.list(environment_name="dev")
            ```

            By default, all named Volumes are returned, newest to oldest. It's also possible to limit the
            number of results and to filter by creation date:

            ```python
            volumes = modal.Volume.objects.list(max_objects=10, created_before="2025-01-01")
            ```
        """
        ...

    async def delete(
        self,
        name: str,
        *,
        allow_missing: bool = False,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
    ):
        """Delete a named Volume entirely (not individual files).

        Deletion is irreversible and affects any Apps using this Volume.

        Added in v1.1.2.

        Args:
            name: Name of the Volume to delete.
            allow_missing: If True, do nothing when the Volume does not exist.
            environment_name: Environment to delete from; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.

        Examples:
            ```python notest
            await modal.Volume.objects.delete("my-volume")
            ```

            Volumes will be deleted from the active environment, or another one can be specified:

            ```python notest
            await modal.Volume.objects.delete("my-volume", environment_name="dev")
            ```
        """
        ...

class VolumeManager:
    """Namespace with methods for managing named Volume objects."""
    def __init__(self, /, *args, **kwargs):
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    class __create_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            name: str,
            *,
            version: typing.Optional[int] = None,
            allow_existing: bool = False,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        ) -> None:
            """Create a new named Volume in the workspace environment.

            This does not return a local handle; use `modal.Volume.from_name` to look up the Volume after creation.

            Added in v1.1.2.

            Args:
                name: Name for the new Volume.
                version: Optional VolumeFS backend version (1 or 2); experimental.
                allow_existing: If True, do nothing when a Volume with this name already exists.
                environment_name: Environment to create in; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.
                experimental_options: Experimental options to create Volume with.

            Examples:
                ```python notest
                modal.Volume.objects.create("my-volume")
                ```

                Volumes will be created in the active environment, or another one can be specified:

                ```python notest
                modal.Volume.objects.create("my-volume", environment_name="dev")
                ```

                By default, an error is raised if the Volume already exists; `allow_existing=True` makes that case a no-op:

                ```python notest
                modal.Volume.objects.create("my-volume", allow_existing=True)
                ```

                Note that this method does not return a local instance of the Volume. You can use
                `modal.Volume.from_name` to perform a lookup after creation.
            """
            ...

        async def aio(
            self,
            /,
            name: str,
            *,
            version: typing.Optional[int] = None,
            allow_existing: bool = False,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        ) -> None:
            """Create a new named Volume in the workspace environment.

            This does not return a local handle; use `modal.Volume.from_name` to look up the Volume after creation.

            Added in v1.1.2.

            Args:
                name: Name for the new Volume.
                version: Optional VolumeFS backend version (1 or 2); experimental.
                allow_existing: If True, do nothing when a Volume with this name already exists.
                environment_name: Environment to create in; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.
                experimental_options: Experimental options to create Volume with.

            Examples:
                ```python notest
                modal.Volume.objects.create("my-volume")
                ```

                Volumes will be created in the active environment, or another one can be specified:

                ```python notest
                modal.Volume.objects.create("my-volume", environment_name="dev")
                ```

                By default, an error is raised if the Volume already exists; `allow_existing=True` makes that case a no-op:

                ```python notest
                modal.Volume.objects.create("my-volume", allow_existing=True)
                ```

                Note that this method does not return a local instance of the Volume. You can use
                `modal.Volume.from_name` to perform a lookup after creation.
            """
            ...

    create: __create_spec

    class __list_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            max_objects: typing.Optional[int] = None,
            created_before: typing.Union[datetime.datetime, str, None] = None,
            environment_name: str = "",
            client: typing.Optional[modal.client.Client] = None,
        ) -> list[Volume]:
            """List named Volumes in the workspace environment as hydrated handles.

            Results are ordered newest to oldest. By default, all matching Volumes are returned.

            Added in v1.1.2.

            Args:
                max_objects: Maximum number of Volumes to return.
                created_before: Only include Volumes created before this time (datetime or ISO date string).
                environment_name: Environment to list from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                Hydrated `Volume` objects for each named Volume in the listing.

            Examples:
                ```python
                volumes = modal.Volume.objects.list()
                print([v.name for v in volumes])
                ```

                Volumes will be retrieved from the active environment, or another one can be specified:

                ```python notest
                dev_volumes = modal.Volume.objects.list(environment_name="dev")
                ```

                By default, all named Volumes are returned, newest to oldest. It's also possible to limit the
                number of results and to filter by creation date:

                ```python
                volumes = modal.Volume.objects.list(max_objects=10, created_before="2025-01-01")
                ```
            """
            ...

        async def aio(
            self,
            /,
            *,
            max_objects: typing.Optional[int] = None,
            created_before: typing.Union[datetime.datetime, str, None] = None,
            environment_name: str = "",
            client: typing.Optional[modal.client.Client] = None,
        ) -> list[Volume]:
            """List named Volumes in the workspace environment as hydrated handles.

            Results are ordered newest to oldest. By default, all matching Volumes are returned.

            Added in v1.1.2.

            Args:
                max_objects: Maximum number of Volumes to return.
                created_before: Only include Volumes created before this time (datetime or ISO date string).
                environment_name: Environment to list from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                Hydrated `Volume` objects for each named Volume in the listing.

            Examples:
                ```python
                volumes = modal.Volume.objects.list()
                print([v.name for v in volumes])
                ```

                Volumes will be retrieved from the active environment, or another one can be specified:

                ```python notest
                dev_volumes = modal.Volume.objects.list(environment_name="dev")
                ```

                By default, all named Volumes are returned, newest to oldest. It's also possible to limit the
                number of results and to filter by creation date:

                ```python
                volumes = modal.Volume.objects.list(max_objects=10, created_before="2025-01-01")
                ```
            """
            ...

    list: __list_spec

    class __delete_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            name: str,
            *,
            allow_missing: bool = False,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ):
            """Delete a named Volume entirely (not individual files).

            Deletion is irreversible and affects any Apps using this Volume.

            Added in v1.1.2.

            Args:
                name: Name of the Volume to delete.
                allow_missing: If True, do nothing when the Volume does not exist.
                environment_name: Environment to delete from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Examples:
                ```python notest
                await modal.Volume.objects.delete("my-volume")
                ```

                Volumes will be deleted from the active environment, or another one can be specified:

                ```python notest
                await modal.Volume.objects.delete("my-volume", environment_name="dev")
                ```
            """
            ...

        async def aio(
            self,
            /,
            name: str,
            *,
            allow_missing: bool = False,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ):
            """Delete a named Volume entirely (not individual files).

            Deletion is irreversible and affects any Apps using this Volume.

            Added in v1.1.2.

            Args:
                name: Name of the Volume to delete.
                allow_missing: If True, do nothing when the Volume does not exist.
                environment_name: Environment to delete from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Examples:
                ```python notest
                await modal.Volume.objects.delete("my-volume")
                ```

                Volumes will be deleted from the active environment, or another one can be specified:

                ```python notest
                await modal.Volume.objects.delete("my-volume", environment_name="dev")
                ```
            """
            ...

    delete: __delete_spec

class _VolumeMountOptions:
    """_VolumeMountOptions(read_only: bool = False, sub_path: str | None = None)"""

    read_only: bool
    sub_path: typing.Optional[str]

    def __init__(self, read_only: bool = False, sub_path: typing.Optional[str] = None) -> None:
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

def _volume_to_mount_proto(path: str, volume: _Volume) -> modal_proto.api_pb2.VolumeMount:
    """Convert a Volume to a VolumeMount proto for use in Function/Sandbox definitions."""
    ...

class _Volume(modal._object._Object):
    """A writeable volume that can be used to share files between one or more Modal functions.

    The contents of a volume is exposed as a filesystem. You can use it to share data between different functions, or
    to persist durable state across several instances of the same function.

    Unlike a networked filesystem, you need to explicitly reload the volume to see changes made since it was mounted.
    Similarly, you need to explicitly commit any changes you make to the volume for the changes to become visible
    outside the current container.

    Concurrent modification is supported, but concurrent modifications of the same files should be avoided! Last write
    wins in case of concurrent modification of the same file - any data the last writer didn't have when committing
    changes will be lost!

    As a result, volumes are typically not a good fit for use cases where you need to make concurrent modifications to
    the same file (nor is distributed file locking supported).

    Volumes can only be reloaded if there are no open files for the volume - attempting to reload with open files
    will result in an error.

    Examples:
        ```python
        import modal

        app = modal.App()
        volume = modal.Volume.from_name("my-persisted-volume", create_if_missing=True)

        @app.function(volumes={"/root/foo": volume})
        def f():
            with open("/root/foo/bar.txt", "w") as f:
                f.write("hello")
            volume.commit()  # Persist changes

        @app.function(volumes={"/root/foo": volume})
        def g():
            volume.reload()  # Fetch latest changes
            with open("/root/foo/bar.txt", "r") as f:
                print(f.read())
        ```
    """

    _lock: typing.Optional[asyncio.locks.Lock]
    _metadata: typing.Optional[modal_proto.api_pb2.VolumeMetadata]
    _mount_options: typing.Optional[_VolumeMountOptions]
    _read_only: bool

    def _initialize_from_empty(self): ...
    def _initialize_from_other(self, other): ...
    @synchronicity.classproperty
    @classmethod
    def objects(cls) -> _VolumeManager: ...
    @property
    def name(self) -> typing.Optional[str]: ...
    def read_only(self) -> _Volume:
        """mdmd:hidden"""
        ...

    def with_mount_options(
        self,
        *,
        read_only: typing.Optional[bool] = None,
        sub_path: typing.Union[str, pathlib.PurePosixPath, None] = None,
    ) -> _Volume:
        """Configure options used when mounting this Volume.

        Note that these options are not properties stored with the Volume itself - they can be individually configured
        for each Volume - container association.

        Args:
            read_only: Set this to True to make the Volume read only from within containers.
            sub_path:
                Only mount this sub_path directory from the Volume. If the directory doesn't exist in the Volume,
                it will be created when the container starts up.

        Returns:
            A `Volume` handle with the mount options applied.

        Examples:
            To mount a volume in read-only mode:

            ```python
            import modal

            volume = modal.Volume.from_name("my-volume")

            @app.function(volumes={"/mnt": volume.with_mount_options(read_only=True)})
            def f():
                return os.mkdir("/mnt/foo")  # not possible!
            ```

            To mount only part of a Volume using sub_path:

            ```python
            import modal

            volume = modal.Volume.from_name("my-volume")

            @app.function(volumes={"/user_data": volume.with_mount_options(sub_path="/users/my_user")})
            def f():
                return os.listdir("/user_data")  # lists data from /users/my_user
            ```
        """
        ...

    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self) -> typing.Optional[google.protobuf.message.Message]: ...
    async def _get_lock(self): ...
    @property
    def _is_v1(self) -> bool: ...
    @staticmethod
    def from_name(
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        create_if_missing: bool = False,
        version: typing.Optional[int] = None,
        create_options: typing.Optional[modal.types.VolumeCreateOptions] = None,
        client: typing.Optional[modal.client._Client] = None,
    ) -> _Volume:
        """Reference a Volume by name, optionally creating it on the server first.

        Hydration is lazy: metadata is fetched from Modal the first time the handle is used.

        Args:
            name: Deployment name of the Volume.
            environment_name: Environment to resolve the name in; defaults to the active environment.
            create_if_missing: If True, create the Volume when it does not already exist.
            version: Optional VolumeFS backend version; must match an existing Volume when set.
            create_options: Applied when creating the Volume. If an existing Volume, validates options are consistent.
            client: Modal client to use for loading; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Volume` handle (possibly not yet hydrated).

        Examples:
            ```python
            vol = modal.Volume.from_name("my-volume", create_if_missing=True)

            app = modal.App()

            @app.function(volumes={"/data": vol})
            def f():
                pass
            ```
        """
        ...

    @staticmethod
    def from_id(volume_id: str, client: typing.Optional[modal.client._Client] = None) -> _Volume:
        """Construct a Volume from an id and look up the Volume metadata.

        This is a lazy method that defers hydrating the local
        object with metadata from Modal servers until the first
        time it is actually used.

        The ID of a Volume object can be accessed using `.object_id`.

        Args:
            volume_id: Volume object ID to attach to.
            client: Modal client to use for loading; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Volume` handle (possibly not yet hydrated).

        Examples:
            ```python notest
            @app.function()
            def my_worker(volume_id: str):
                vol = modal.Volume.from_id(volume_id)
                for entry in vol.listdir("/"):
                    print(entry.path)

            with modal.Volume.ephemeral() as vol:
                my_worker.remote(vol.object_id)
            ```
        """
        ...

    @classmethod
    def ephemeral(
        cls: type[_Volume],
        client: typing.Optional[modal.client._Client] = None,
        environment_name: typing.Optional[str] = None,
        version: typing.Optional[int] = None,
        create_options: typing.Optional[modal.types.VolumeCreateOptions] = None,
        _heartbeat_sleep: float = 300,
    ) -> typing.AsyncContextManager[_Volume]:
        """Create an anonymous Volume that exists for the duration of the context manager.

        Args:
            client: Modal client to use; defaults to `Client.from_env()` when omitted.
            environment_name: Environment for the ephemeral Volume; defaults to the active environment.
            version: Optional VolumeFS backend version for the ephemeral Volume.
            create_options: Options applied when creating the ephemeral Volume.

        Examples:
            ```python
            import modal

            with modal.Volume.ephemeral() as vol:
                assert vol.listdir("/") == []
            ```

            ```python notest
            async with modal.Volume.ephemeral() as vol:
                assert await vol.listdir("/") == []
            ```
        """
        ...

    async def info(self) -> modal.types.VolumeInfo:
        """Return information about the Volume object."""
        ...

    async def _do_reload(self, lock=True): ...
    async def commit(self):
        """Commit changes to a mounted volume.

        If successful, the changes made are now persisted in durable storage and available to other containers accessing
        the volume.
        """
        ...

    async def reload(self):
        """Make latest committed state of volume available in the running container.

        Any uncommitted changes to the volume, such as new or modified files, may implicitly be committed when
        reloading.

        Reloading will fail if there are open files for the volume.
        """
        ...

    def iterdir(self, path: str, *, recursive: bool = True) -> collections.abc.AsyncIterator[modal.types.FileEntry]:
        """Iterate over all files in a directory in the volume.

        Passing a directory path lists all files in the directory. For a file path, return only that
        file's description. If `recursive` is set to True, list all files and folders under the path
        recursively.
        """
        ...

    async def listdir(self, path: str, *, recursive: bool = False) -> list[modal.types.FileEntry]:
        """List all files under a path prefix in the modal.Volume.

        Passing a directory path lists all files in the directory. For a file path, return only that
        file's description. If `recursive` is set to True, list all files and folders under the path
        recursively.
        """
        ...

    def read_file(self, path: str) -> collections.abc.AsyncGenerator[bytes, None]:
        """Read a file from the modal.Volume.

        Note - this function is primarily intended to be used outside of a Modal App.
        For more information on downloading files from a Modal Volume, see
        [the guide](https://modal.com/docs/guide/volumes).

        Args:
            path: Path to the file inside the Volume.

        Examples:
            ```python notest
            vol = modal.Volume.from_name("my-modal-volume")
            data = b""
            for chunk in vol.read_file("1mb.csv"):
                data += chunk
            print(len(data))  # == 1024 * 1024
            ```
        """
        ...

    async def read_file_into_fileobj(
        self,
        path: str,
        fileobj: typing.IO[bytes],
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
    ) -> int:
        """mdmd:hidden
        Read volume file into file-like IO object.
        """
        ...

    async def _read_file_into_fileobj(
        self,
        path: str,
        fileobj: typing.IO[bytes],
        concurrency: typing.Optional[int] = None,
        download_semaphore: typing.Optional[asyncio.locks.Semaphore] = None,
        rpc_semaphore: typing.Optional[asyncio.locks.Semaphore] = None,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
    ) -> int: ...
    async def remove_file(self, path: str, recursive: bool = False) -> None:
        """Remove a file or directory from a volume."""
        ...

    async def copy_files(
        self, src_paths: collections.abc.Sequence[str], dst_path: str, recursive: bool = False
    ) -> None:
        """Copy files within the volume from src_paths to dst_path.
        The semantics of the copy operation follow those of the UNIX cp command.

        The `src_paths` parameter is a list. If you want to copy a single file, you should pass a list with a
        single element.

        `src_paths` and `dst_path` should refer to the desired location *inside* the volume. You do not need to prepend
        the volume mount path.

        Args:
            src_paths: Source paths inside the Volume (list of one or more paths).
            dst_path: Destination path inside the Volume (file or directory, following ``cp`` semantics).
            recursive: Whether to copy directories recursively (V2 volumes only).

        Examples:
            ```python notest
            vol = modal.Volume.from_name("my-modal-volume")

            vol.copy_files(["bar/example.txt"], "bar2")
            vol.copy_files(["bar/example.txt"], "bar/example2.txt")
            ```

        Note that if the volume is already mounted on the Modal function, you should use normal filesystem operations
        like `os.rename()` and then `commit()` the volume. The `copy_files()` method is useful when you don't have
        the volume mounted as a filesystem, e.g. when running a script on your local computer.
        """
        ...

    def batch_upload(self, force: bool = False) -> typing.AsyncContextManager[_AbstractVolumeUploadContextManager]:
        """Initiate a batched upload to a volume.

        To allow overwriting existing files, set `force` to `True` (you cannot overwrite existing directories with
        uploaded files regardless).

        Args:
            force: If True, allow overwriting existing files with uploads (not directories).

        Examples:
            ```python notest
            vol = modal.Volume.from_name("my-modal-volume")

            with vol.batch_upload() as batch:
                batch.put_file("local-path.txt", "/remote-path.txt")
                batch.put_directory("/local/directory/", "/remote/directory")
                batch.put_file(io.BytesIO(b"some data"), "/foobar")
            ```
        """
        ...

    async def _instance_delete(self): ...
    @staticmethod
    async def rename(
        old_name: str,
        new_name: str,
        *,
        client: typing.Optional[modal.client._Client] = None,
        environment_name: typing.Optional[str] = None,
    ): ...

class Volume(modal.object.Object):
    """A writeable volume that can be used to share files between one or more Modal functions.

    The contents of a volume is exposed as a filesystem. You can use it to share data between different functions, or
    to persist durable state across several instances of the same function.

    Unlike a networked filesystem, you need to explicitly reload the volume to see changes made since it was mounted.
    Similarly, you need to explicitly commit any changes you make to the volume for the changes to become visible
    outside the current container.

    Concurrent modification is supported, but concurrent modifications of the same files should be avoided! Last write
    wins in case of concurrent modification of the same file - any data the last writer didn't have when committing
    changes will be lost!

    As a result, volumes are typically not a good fit for use cases where you need to make concurrent modifications to
    the same file (nor is distributed file locking supported).

    Volumes can only be reloaded if there are no open files for the volume - attempting to reload with open files
    will result in an error.

    Examples:
        ```python
        import modal

        app = modal.App()
        volume = modal.Volume.from_name("my-persisted-volume", create_if_missing=True)

        @app.function(volumes={"/root/foo": volume})
        def f():
            with open("/root/foo/bar.txt", "w") as f:
                f.write("hello")
            volume.commit()  # Persist changes

        @app.function(volumes={"/root/foo": volume})
        def g():
            volume.reload()  # Fetch latest changes
            with open("/root/foo/bar.txt", "r") as f:
                print(f.read())
        ```
    """

    _lock: typing.Optional[asyncio.locks.Lock]
    _metadata: typing.Optional[modal_proto.api_pb2.VolumeMetadata]
    _mount_options: typing.Optional[_VolumeMountOptions]
    _read_only: bool

    def __init__(self, *args, **kwargs):
        """mdmd:hidden"""
        ...

    def _initialize_from_empty(self): ...
    def _initialize_from_other(self, other): ...
    @synchronicity.classproperty
    @classmethod
    def objects(cls) -> VolumeManager: ...
    @property
    def name(self) -> typing.Optional[str]: ...
    def read_only(self) -> Volume:
        """mdmd:hidden"""
        ...

    def with_mount_options(
        self,
        *,
        read_only: typing.Optional[bool] = None,
        sub_path: typing.Union[str, pathlib.PurePosixPath, None] = None,
    ) -> Volume:
        """Configure options used when mounting this Volume.

        Note that these options are not properties stored with the Volume itself - they can be individually configured
        for each Volume - container association.

        Args:
            read_only: Set this to True to make the Volume read only from within containers.
            sub_path:
                Only mount this sub_path directory from the Volume. If the directory doesn't exist in the Volume,
                it will be created when the container starts up.

        Returns:
            A `Volume` handle with the mount options applied.

        Examples:
            To mount a volume in read-only mode:

            ```python
            import modal

            volume = modal.Volume.from_name("my-volume")

            @app.function(volumes={"/mnt": volume.with_mount_options(read_only=True)})
            def f():
                return os.mkdir("/mnt/foo")  # not possible!
            ```

            To mount only part of a Volume using sub_path:

            ```python
            import modal

            volume = modal.Volume.from_name("my-volume")

            @app.function(volumes={"/user_data": volume.with_mount_options(sub_path="/users/my_user")})
            def f():
                return os.listdir("/user_data")  # lists data from /users/my_user
            ```
        """
        ...

    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self) -> typing.Optional[google.protobuf.message.Message]: ...

    class ___get_lock_spec(typing_extensions.Protocol):
        def __call__(self, /): ...
        async def aio(self, /): ...

    _get_lock: ___get_lock_spec

    @property
    def _is_v1(self) -> bool: ...
    @staticmethod
    def from_name(
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        create_if_missing: bool = False,
        version: typing.Optional[int] = None,
        create_options: typing.Optional[modal.types.VolumeCreateOptions] = None,
        client: typing.Optional[modal.client.Client] = None,
    ) -> Volume:
        """Reference a Volume by name, optionally creating it on the server first.

        Hydration is lazy: metadata is fetched from Modal the first time the handle is used.

        Args:
            name: Deployment name of the Volume.
            environment_name: Environment to resolve the name in; defaults to the active environment.
            create_if_missing: If True, create the Volume when it does not already exist.
            version: Optional VolumeFS backend version; must match an existing Volume when set.
            create_options: Applied when creating the Volume. If an existing Volume, validates options are consistent.
            client: Modal client to use for loading; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Volume` handle (possibly not yet hydrated).

        Examples:
            ```python
            vol = modal.Volume.from_name("my-volume", create_if_missing=True)

            app = modal.App()

            @app.function(volumes={"/data": vol})
            def f():
                pass
            ```
        """
        ...

    @staticmethod
    def from_id(volume_id: str, client: typing.Optional[modal.client.Client] = None) -> Volume:
        """Construct a Volume from an id and look up the Volume metadata.

        This is a lazy method that defers hydrating the local
        object with metadata from Modal servers until the first
        time it is actually used.

        The ID of a Volume object can be accessed using `.object_id`.

        Args:
            volume_id: Volume object ID to attach to.
            client: Modal client to use for loading; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Volume` handle (possibly not yet hydrated).

        Examples:
            ```python notest
            @app.function()
            def my_worker(volume_id: str):
                vol = modal.Volume.from_id(volume_id)
                for entry in vol.listdir("/"):
                    print(entry.path)

            with modal.Volume.ephemeral() as vol:
                my_worker.remote(vol.object_id)
            ```
        """
        ...

    class __ephemeral_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            version: typing.Optional[int] = None,
            create_options: typing.Optional[modal.types.VolumeCreateOptions] = None,
            _heartbeat_sleep: float = 300,
        ) -> synchronicity.combined_types.AsyncAndBlockingContextManager[Volume]:
            """Create an anonymous Volume that exists for the duration of the context manager.

            Args:
                client: Modal client to use; defaults to `Client.from_env()` when omitted.
                environment_name: Environment for the ephemeral Volume; defaults to the active environment.
                version: Optional VolumeFS backend version for the ephemeral Volume.
                create_options: Options applied when creating the ephemeral Volume.

            Examples:
                ```python
                import modal

                with modal.Volume.ephemeral() as vol:
                    assert vol.listdir("/") == []
                ```

                ```python notest
                async with modal.Volume.ephemeral() as vol:
                    assert await vol.listdir("/") == []
                ```
            """
            ...

        def aio(
            self,
            /,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            version: typing.Optional[int] = None,
            create_options: typing.Optional[modal.types.VolumeCreateOptions] = None,
            _heartbeat_sleep: float = 300,
        ) -> typing.AsyncContextManager[Volume]:
            """Create an anonymous Volume that exists for the duration of the context manager.

            Args:
                client: Modal client to use; defaults to `Client.from_env()` when omitted.
                environment_name: Environment for the ephemeral Volume; defaults to the active environment.
                version: Optional VolumeFS backend version for the ephemeral Volume.
                create_options: Options applied when creating the ephemeral Volume.

            Examples:
                ```python
                import modal

                with modal.Volume.ephemeral() as vol:
                    assert vol.listdir("/") == []
                ```

                ```python notest
                async with modal.Volume.ephemeral() as vol:
                    assert await vol.listdir("/") == []
                ```
            """
            ...

    ephemeral: typing.ClassVar[__ephemeral_spec]

    class __info_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal.types.VolumeInfo:
            """Return information about the Volume object."""
            ...

        async def aio(self, /) -> modal.types.VolumeInfo:
            """Return information about the Volume object."""
            ...

    info: __info_spec

    class ___do_reload_spec(typing_extensions.Protocol):
        def __call__(self, /, lock=True): ...
        async def aio(self, /, lock=True): ...

    _do_reload: ___do_reload_spec

    class __commit_spec(typing_extensions.Protocol):
        def __call__(self, /):
            """Commit changes to a mounted volume.

            If successful, the changes made are now persisted in durable storage and available to other containers accessing
            the volume.
            """
            ...

        async def aio(self, /):
            """Commit changes to a mounted volume.

            If successful, the changes made are now persisted in durable storage and available to other containers accessing
            the volume.
            """
            ...

    commit: __commit_spec

    class __reload_spec(typing_extensions.Protocol):
        def __call__(self, /):
            """Make latest committed state of volume available in the running container.

            Any uncommitted changes to the volume, such as new or modified files, may implicitly be committed when
            reloading.

            Reloading will fail if there are open files for the volume.
            """
            ...

        async def aio(self, /):
            """Make latest committed state of volume available in the running container.

            Any uncommitted changes to the volume, such as new or modified files, may implicitly be committed when
            reloading.

            Reloading will fail if there are open files for the volume.
            """
            ...

    reload: __reload_spec

    class __iterdir_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str, *, recursive: bool = True) -> typing.Iterator[modal.types.FileEntry]:
            """Iterate over all files in a directory in the volume.

            Passing a directory path lists all files in the directory. For a file path, return only that
            file's description. If `recursive` is set to True, list all files and folders under the path
            recursively.
            """
            ...

        def aio(self, /, path: str, *, recursive: bool = True) -> collections.abc.AsyncIterator[modal.types.FileEntry]:
            """Iterate over all files in a directory in the volume.

            Passing a directory path lists all files in the directory. For a file path, return only that
            file's description. If `recursive` is set to True, list all files and folders under the path
            recursively.
            """
            ...

    iterdir: __iterdir_spec

    class __listdir_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str, *, recursive: bool = False) -> list[modal.types.FileEntry]:
            """List all files under a path prefix in the modal.Volume.

            Passing a directory path lists all files in the directory. For a file path, return only that
            file's description. If `recursive` is set to True, list all files and folders under the path
            recursively.
            """
            ...

        async def aio(self, /, path: str, *, recursive: bool = False) -> list[modal.types.FileEntry]:
            """List all files under a path prefix in the modal.Volume.

            Passing a directory path lists all files in the directory. For a file path, return only that
            file's description. If `recursive` is set to True, list all files and folders under the path
            recursively.
            """
            ...

    listdir: __listdir_spec

    class __read_file_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str) -> typing.Generator[bytes, None, None]:
            """Read a file from the modal.Volume.

            Note - this function is primarily intended to be used outside of a Modal App.
            For more information on downloading files from a Modal Volume, see
            [the guide](https://modal.com/docs/guide/volumes).

            Args:
                path: Path to the file inside the Volume.

            Examples:
                ```python notest
                vol = modal.Volume.from_name("my-modal-volume")
                data = b""
                for chunk in vol.read_file("1mb.csv"):
                    data += chunk
                print(len(data))  # == 1024 * 1024
                ```
            """
            ...

        def aio(self, /, path: str) -> collections.abc.AsyncGenerator[bytes, None]:
            """Read a file from the modal.Volume.

            Note - this function is primarily intended to be used outside of a Modal App.
            For more information on downloading files from a Modal Volume, see
            [the guide](https://modal.com/docs/guide/volumes).

            Args:
                path: Path to the file inside the Volume.

            Examples:
                ```python notest
                vol = modal.Volume.from_name("my-modal-volume")
                data = b""
                for chunk in vol.read_file("1mb.csv"):
                    data += chunk
                print(len(data))  # == 1024 * 1024
                ```
            """
            ...

    read_file: __read_file_spec

    class __read_file_into_fileobj_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            path: str,
            fileobj: typing.IO[bytes],
            progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        ) -> int:
            """mdmd:hidden
            Read volume file into file-like IO object.
            """
            ...

        async def aio(
            self,
            /,
            path: str,
            fileobj: typing.IO[bytes],
            progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        ) -> int:
            """mdmd:hidden
            Read volume file into file-like IO object.
            """
            ...

    read_file_into_fileobj: __read_file_into_fileobj_spec

    class ___read_file_into_fileobj_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            path: str,
            fileobj: typing.IO[bytes],
            concurrency: typing.Optional[int] = None,
            download_semaphore: typing.Optional[asyncio.locks.Semaphore] = None,
            rpc_semaphore: typing.Optional[asyncio.locks.Semaphore] = None,
            progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        ) -> int: ...
        async def aio(
            self,
            /,
            path: str,
            fileobj: typing.IO[bytes],
            concurrency: typing.Optional[int] = None,
            download_semaphore: typing.Optional[asyncio.locks.Semaphore] = None,
            rpc_semaphore: typing.Optional[asyncio.locks.Semaphore] = None,
            progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        ) -> int: ...

    _read_file_into_fileobj: ___read_file_into_fileobj_spec

    class __remove_file_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str, recursive: bool = False) -> None:
            """Remove a file or directory from a volume."""
            ...

        async def aio(self, /, path: str, recursive: bool = False) -> None:
            """Remove a file or directory from a volume."""
            ...

    remove_file: __remove_file_spec

    class __copy_files_spec(typing_extensions.Protocol):
        def __call__(self, /, src_paths: collections.abc.Sequence[str], dst_path: str, recursive: bool = False) -> None:
            """Copy files within the volume from src_paths to dst_path.
            The semantics of the copy operation follow those of the UNIX cp command.

            The `src_paths` parameter is a list. If you want to copy a single file, you should pass a list with a
            single element.

            `src_paths` and `dst_path` should refer to the desired location *inside* the volume. You do not need to prepend
            the volume mount path.

            Args:
                src_paths: Source paths inside the Volume (list of one or more paths).
                dst_path: Destination path inside the Volume (file or directory, following ``cp`` semantics).
                recursive: Whether to copy directories recursively (V2 volumes only).

            Examples:
                ```python notest
                vol = modal.Volume.from_name("my-modal-volume")

                vol.copy_files(["bar/example.txt"], "bar2")
                vol.copy_files(["bar/example.txt"], "bar/example2.txt")
                ```

            Note that if the volume is already mounted on the Modal function, you should use normal filesystem operations
            like `os.rename()` and then `commit()` the volume. The `copy_files()` method is useful when you don't have
            the volume mounted as a filesystem, e.g. when running a script on your local computer.
            """
            ...

        async def aio(
            self, /, src_paths: collections.abc.Sequence[str], dst_path: str, recursive: bool = False
        ) -> None:
            """Copy files within the volume from src_paths to dst_path.
            The semantics of the copy operation follow those of the UNIX cp command.

            The `src_paths` parameter is a list. If you want to copy a single file, you should pass a list with a
            single element.

            `src_paths` and `dst_path` should refer to the desired location *inside* the volume. You do not need to prepend
            the volume mount path.

            Args:
                src_paths: Source paths inside the Volume (list of one or more paths).
                dst_path: Destination path inside the Volume (file or directory, following ``cp`` semantics).
                recursive: Whether to copy directories recursively (V2 volumes only).

            Examples:
                ```python notest
                vol = modal.Volume.from_name("my-modal-volume")

                vol.copy_files(["bar/example.txt"], "bar2")
                vol.copy_files(["bar/example.txt"], "bar/example2.txt")
                ```

            Note that if the volume is already mounted on the Modal function, you should use normal filesystem operations
            like `os.rename()` and then `commit()` the volume. The `copy_files()` method is useful when you don't have
            the volume mounted as a filesystem, e.g. when running a script on your local computer.
            """
            ...

    copy_files: __copy_files_spec

    class __batch_upload_spec(typing_extensions.Protocol):
        def __call__(
            self, /, force: bool = False
        ) -> synchronicity.combined_types.AsyncAndBlockingContextManager[AbstractVolumeUploadContextManager]:
            """Initiate a batched upload to a volume.

            To allow overwriting existing files, set `force` to `True` (you cannot overwrite existing directories with
            uploaded files regardless).

            Args:
                force: If True, allow overwriting existing files with uploads (not directories).

            Examples:
                ```python notest
                vol = modal.Volume.from_name("my-modal-volume")

                with vol.batch_upload() as batch:
                    batch.put_file("local-path.txt", "/remote-path.txt")
                    batch.put_directory("/local/directory/", "/remote/directory")
                    batch.put_file(io.BytesIO(b"some data"), "/foobar")
                ```
            """
            ...

        def aio(self, /, force: bool = False) -> typing.AsyncContextManager[AbstractVolumeUploadContextManager]:
            """Initiate a batched upload to a volume.

            To allow overwriting existing files, set `force` to `True` (you cannot overwrite existing directories with
            uploaded files regardless).

            Args:
                force: If True, allow overwriting existing files with uploads (not directories).

            Examples:
                ```python notest
                vol = modal.Volume.from_name("my-modal-volume")

                with vol.batch_upload() as batch:
                    batch.put_file("local-path.txt", "/remote-path.txt")
                    batch.put_directory("/local/directory/", "/remote/directory")
                    batch.put_file(io.BytesIO(b"some data"), "/foobar")
                ```
            """
            ...

    batch_upload: __batch_upload_spec

    class ___instance_delete_spec(typing_extensions.Protocol):
        def __call__(self, /): ...
        async def aio(self, /): ...

    _instance_delete: ___instance_delete_spec

    class __rename_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            old_name: str,
            new_name: str,
            *,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
        ): ...
        async def aio(
            self,
            /,
            old_name: str,
            new_name: str,
            *,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
        ): ...

    rename: typing.ClassVar[__rename_spec]

class _AbstractVolumeUploadContextManager:
    async def __aenter__(self): ...
    async def __aexit__(self, exc_type, exc_val, exc_tb): ...
    def put_file(
        self,
        local_file: typing.Union[pathlib.Path, str, typing.BinaryIO, _io.BytesIO],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        mode: typing.Optional[int] = None,
    ): ...
    def put_directory(
        self,
        local_path: typing.Union[pathlib.Path, str],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        recursive: bool = True,
    ): ...
    @staticmethod
    def resolve(
        version: int,
        object_id: str,
        client,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        force: bool = False,
    ) -> _AbstractVolumeUploadContextManager: ...

class AbstractVolumeUploadContextManager:
    def __init__(self, /, *args, **kwargs):
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __enter__(self): ...
    async def __aenter__(self): ...
    def __exit__(self, exc_type, exc_val, exc_tb): ...
    async def __aexit__(self, exc_type, exc_val, exc_tb): ...
    def put_file(
        self,
        local_file: typing.Union[pathlib.Path, str, typing.BinaryIO, _io.BytesIO],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        mode: typing.Optional[int] = None,
    ): ...
    def put_directory(
        self,
        local_path: typing.Union[pathlib.Path, str],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        recursive: bool = True,
    ): ...
    @staticmethod
    def resolve(
        version: int,
        object_id: str,
        client,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        force: bool = False,
    ) -> AbstractVolumeUploadContextManager: ...

class _VolumeUploadContextManager(_AbstractVolumeUploadContextManager):
    """Context manager for batch-uploading files to a Volume."""

    _volume_id: str
    _client: modal.client._Client
    _force: bool
    progress_cb: collections.abc.Callable[..., typing.Any]
    _upload_generators: list[
        collections.abc.Generator[collections.abc.Callable[[], modal._utils.blob_utils.FileUploadSpec], None, None]
    ]

    def __init__(
        self,
        volume_id: str,
        client: modal.client._Client,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        force: bool = False,
    ):
        """mdmd:hidden"""
        ...

    async def __aenter__(self): ...
    async def __aexit__(self, exc_type, exc_val, exc_tb): ...
    def put_file(
        self,
        local_file: typing.Union[pathlib.Path, str, typing.BinaryIO, _io.BytesIO],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        mode: typing.Optional[int] = None,
    ):
        """Upload a file from a local file or file-like object.

        Will create any needed parent directories automatically.

        If `local_file` is a file-like object it must remain readable for the lifetime of the batch.
        """
        ...

    def put_directory(
        self,
        local_path: typing.Union[pathlib.Path, str],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        recursive: bool = True,
    ):
        """Upload all files in a local directory.

        Will create any needed parent directories automatically.
        """
        ...

    async def _upload_file(
        self, file_spec: modal._utils.blob_utils.FileUploadSpec
    ) -> modal_proto.api_pb2.MountFile: ...

class VolumeUploadContextManager(AbstractVolumeUploadContextManager):
    """Context manager for batch-uploading files to a Volume."""

    _volume_id: str
    _client: modal.client.Client
    _force: bool
    progress_cb: collections.abc.Callable[..., typing.Any]
    _upload_generators: list[
        collections.abc.Generator[collections.abc.Callable[[], modal._utils.blob_utils.FileUploadSpec], None, None]
    ]

    def __init__(
        self,
        volume_id: str,
        client: modal.client.Client,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        force: bool = False,
    ):
        """mdmd:hidden"""
        ...

    def __enter__(self): ...
    async def __aenter__(self): ...
    def __exit__(self, exc_type, exc_val, exc_tb): ...
    async def __aexit__(self, exc_type, exc_val, exc_tb): ...
    def put_file(
        self,
        local_file: typing.Union[pathlib.Path, str, typing.BinaryIO, _io.BytesIO],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        mode: typing.Optional[int] = None,
    ):
        """Upload a file from a local file or file-like object.

        Will create any needed parent directories automatically.

        If `local_file` is a file-like object it must remain readable for the lifetime of the batch.
        """
        ...

    def put_directory(
        self,
        local_path: typing.Union[pathlib.Path, str],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        recursive: bool = True,
    ):
        """Upload all files in a local directory.

        Will create any needed parent directories automatically.
        """
        ...

    class ___upload_file_spec(typing_extensions.Protocol):
        def __call__(self, /, file_spec: modal._utils.blob_utils.FileUploadSpec) -> modal_proto.api_pb2.MountFile: ...
        async def aio(self, /, file_spec: modal._utils.blob_utils.FileUploadSpec) -> modal_proto.api_pb2.MountFile: ...

    _upload_file: ___upload_file_spec

class _VolumeUploadContextManager2(_AbstractVolumeUploadContextManager):
    """Context manager for batch-uploading files to a Volume version 2."""

    _volume_id: str
    _client: modal.client._Client
    _progress_cb: collections.abc.Callable[..., typing.Any]
    _force: bool
    _hash_concurrency: int
    _put_concurrency: int
    _uploader_generators: list[
        collections.abc.Generator[
            collections.abc.Callable[
                [asyncio.locks.Semaphore], collections.abc.Awaitable[modal._utils.blob_utils.FileUploadSpec2]
            ]
        ]
    ]

    def __init__(
        self,
        volume_id: str,
        client: modal.client._Client,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        force: bool = False,
        hash_concurrency: int = 2,
        put_concurrency: int = 128,
    ):
        """mdmd:hidden"""
        ...

    async def __aenter__(self): ...
    async def __aexit__(self, exc_type, exc_val, exc_tb): ...
    def put_file(
        self,
        local_file: typing.Union[pathlib.Path, str, typing.BinaryIO, _io.BytesIO],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        mode: typing.Optional[int] = None,
    ):
        """Upload a file from a local file or file-like object.

        Will create any needed parent directories automatically.

        If `local_file` is a file-like object it must remain readable for the lifetime of the batch.
        """
        ...

    def put_directory(
        self,
        local_path: typing.Union[pathlib.Path, str],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        recursive: bool = True,
    ):
        """Upload all files in a local directory.

        Will create any needed parent directories automatically.
        """
        ...

    async def _put_file_specs(self, file_specs: list[modal._utils.blob_utils.FileUploadSpec2]): ...

class VolumeUploadContextManager2(AbstractVolumeUploadContextManager):
    """Context manager for batch-uploading files to a Volume version 2."""

    _volume_id: str
    _client: modal.client.Client
    _progress_cb: collections.abc.Callable[..., typing.Any]
    _force: bool
    _hash_concurrency: int
    _put_concurrency: int
    _uploader_generators: list[
        collections.abc.Generator[
            collections.abc.Callable[
                [asyncio.locks.Semaphore], collections.abc.Awaitable[modal._utils.blob_utils.FileUploadSpec2]
            ]
        ]
    ]

    def __init__(
        self,
        volume_id: str,
        client: modal.client.Client,
        progress_cb: typing.Optional[collections.abc.Callable[..., typing.Any]] = None,
        force: bool = False,
        hash_concurrency: int = 2,
        put_concurrency: int = 128,
    ):
        """mdmd:hidden"""
        ...

    def __enter__(self): ...
    async def __aenter__(self): ...
    def __exit__(self, exc_type, exc_val, exc_tb): ...
    async def __aexit__(self, exc_type, exc_val, exc_tb): ...
    def put_file(
        self,
        local_file: typing.Union[pathlib.Path, str, typing.BinaryIO, _io.BytesIO],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        mode: typing.Optional[int] = None,
    ):
        """Upload a file from a local file or file-like object.

        Will create any needed parent directories automatically.

        If `local_file` is a file-like object it must remain readable for the lifetime of the batch.
        """
        ...

    def put_directory(
        self,
        local_path: typing.Union[pathlib.Path, str],
        remote_path: typing.Union[pathlib.PurePosixPath, str],
        recursive: bool = True,
    ):
        """Upload all files in a local directory.

        Will create any needed parent directories automatically.
        """
        ...

    class ___put_file_specs_spec(typing_extensions.Protocol):
        def __call__(self, /, file_specs: list[modal._utils.blob_utils.FileUploadSpec2]): ...
        async def aio(self, /, file_specs: list[modal._utils.blob_utils.FileUploadSpec2]): ...

    _put_file_specs: ___put_file_specs_spec

async def _put_missing_blocks(
    file_specs: list[modal._utils.blob_utils.FileUploadSpec2],
    missing_blocks: list,
    put_responses: dict[bytes, bytes],
    put_concurrency: int,
    progress_cb: collections.abc.Callable[..., typing.Any],
): ...
def _open_files_error_annotation(mount_path: str) -> typing.Optional[str]: ...
