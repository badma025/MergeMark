import _typeshed
import collections.abc
import enum
import google.protobuf.message
import modal._image
import modal._object
import modal._tunnel
import modal._utils.task_command_router_client
import modal.app
import modal.client
import modal.cloud_bucket_mount
import modal.container_process
import modal.file_io
import modal.image
import modal.io_streams
import modal.mount
import modal.network_file_system
import modal.object
import modal.proxy
import modal.sandbox_fs
import modal.secret
import modal.snapshot
import modal.stream_type
import modal.types
import modal.volume
import modal_proto.api_pb2
import modal_proto.task_command_router_pb2
import os
import pathlib
import typing
import typing_extensions

async def _gather_load_with_timings(
    load_coros: collections.abc.Sequence[collections.abc.Awaitable[typing.Any]],
) -> list[tuple[str, float]]:
    """Await all loader coroutines concurrently and return [(object_id, elapsed_seconds)] per load."""
    ...

def _format_sandbox_create_timing_log(
    sandbox_id: str, total_seconds: float, rpc_seconds: float, dep_timings: collections.abc.Sequence[tuple[str, float]]
) -> str:
    """Format the Sandbox create debug log line, listing the slowest deps first."""
    ...

def _validate_sandbox_env(env: dict[str, str]) -> None: ...
def _ttl_to_wire_ttl(ttl: typing.Optional[int]) -> int:
    """Convert a TTL value to the wire format, validating the input."""
    ...

def _validate_experimental_encryption_key(key: typing.Optional[bytes]) -> typing.Optional[bytes]: ...

class SandboxVersion(enum.Enum):
    V1 = 1
    V2 = 2

def _is_v1_sandbox_id(sandbox_id: str) -> bool: ...
def _is_v2_sandbox_id(sandbox_id: str) -> bool: ...
def _get_sandbox_version(sandbox_id: str) -> SandboxVersion: ...
def _result_returncode(result: typing.Optional[modal_proto.api_pb2.GenericResult]) -> typing.Optional[int]: ...
def _validate_exec_args(args: collections.abc.Sequence[str]) -> None: ...

class DefaultSandboxNameOverride(str):
    """A singleton class that represents the default sandbox name override.

    It is used to indicate that the sandbox name should not be overridden.
    """
    def __repr__(self) -> str:
        """Return repr(self)."""
        ...

_DEFAULT_SANDBOX_NAME_OVERRIDE: DefaultSandboxNameOverride

class Probe:
    """Probe configuration for the Sandbox Readiness Probe.

    Examples:
        ```python notest
        # Wait until a file exists.
        readiness_probe = modal.Probe.with_exec(
            "sh", "-c", "test -f /tmp/ready",
        )

        # Wait until a TCP port is accepting connections.
        readiness_probe = modal.Probe.with_tcp(8080)

        app = modal.App.lookup('sandbox-readiness-probe', create_if_missing=True)
        sandbox = modal.Sandbox.create(
            "python3", "-m", "http.server", "8080",
            readiness_probe=readiness_probe,
            app=app,
        )
        sandbox.wait_until_ready()
        ```
    """

    tcp_port: typing.Optional[int]
    exec_argv: typing.Optional[tuple[str, ...]]
    interval_ms: int

    def __post_init__(self): ...
    @classmethod
    def with_tcp(cls, port: int, *, interval_ms: int = 100) -> Probe: ...
    @classmethod
    def with_exec(cls, *argv: str, interval_ms: int = 100) -> Probe: ...
    def _to_proto(self) -> modal_proto.api_pb2.Probe: ...
    def __init__(
        self,
        tcp_port: typing.Optional[int] = None,
        exec_argv: typing.Optional[tuple[str, ...]] = None,
        interval_ms: int = 100,
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

class _Sandbox(modal._object._Object):
    """A `Sandbox` object lets you interact with a running sandbox. This API is similar to Python's
    [asyncio.subprocess.Process](https://docs.python.org/3/library/asyncio-subprocess.html#asyncio.subprocess.Process).

    Refer to the [guide](https://modal.com/docs/guide/sandbox) on how to spawn and use sandboxes.
    """

    _result: typing.Optional[modal_proto.api_pb2.GenericResult]
    _stdout: modal.io_streams._StreamReader[str]
    _stderr: modal.io_streams._StreamReader[str]
    _stdin: modal.io_streams._StreamWriter
    _task_id: typing.Optional[str]
    _tunnels: typing.Optional[dict[int, modal._tunnel.Tunnel]]
    _enable_snapshot: bool
    _command_router_client: typing.Optional[modal._utils.task_command_router_client.TaskCommandRouterClient]
    _attached: bool
    _filesystem: typing.Optional[modal.sandbox_fs._SandboxFilesystem]
    _is_v2: bool
    _app_id: typing.Optional[str]

    @staticmethod
    def _default_pty_info() -> modal_proto.api_pb2.PTYInfo: ...
    @staticmethod
    def _new(
        args: collections.abc.Sequence[str],
        image: modal._image._Image,
        secrets: collections.abc.Collection[modal.secret._Secret],
        name: typing.Optional[str] = None,
        timeout: int = 300,
        idle_timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        gpu: typing.Optional[str] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        mounts: collections.abc.Sequence[modal.mount._Mount] = (),
        network_file_systems: dict[typing.Union[str, os.PathLike], modal.network_file_system._NetworkFileSystem] = {},
        block_network: bool = False,
        outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        volumes: dict[
            typing.Union[str, os.PathLike],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        pty: bool = False,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        encrypted_ports: collections.abc.Sequence[int] = [],
        h2_ports: collections.abc.Sequence[int] = [],
        unencrypted_ports: collections.abc.Sequence[int] = [],
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        readiness_probe: typing.Optional[Probe] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        tags: typing.Optional[dict[str, str]] = None,
        enable_snapshot: bool = False,
        verbose: bool = False,
        custom_domain: typing.Optional[str] = None,
        include_oidc_identity_token: bool = False,
    ) -> _Sandbox:
        """mdmd:hidden"""
        ...

    @staticmethod
    async def create(
        *args: str,
        app: typing.Optional[modal.app._App] = None,
        name: typing.Optional[str] = None,
        tags: typing.Optional[dict[str, str]] = None,
        image: typing.Optional[modal._image._Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        network_file_systems: dict[typing.Union[str, os.PathLike], modal.network_file_system._NetworkFileSystem] = {},
        timeout: int = 300,
        idle_timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        gpu: typing.Optional[str] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        block_network: bool = False,
        outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        volumes: dict[
            typing.Union[str, os.PathLike],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        pty: bool = False,
        encrypted_ports: collections.abc.Sequence[int] = [],
        h2_ports: collections.abc.Sequence[int] = [],
        unencrypted_ports: collections.abc.Sequence[int] = [],
        custom_domain: typing.Optional[str] = None,
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        include_oidc_identity_token: bool = False,
        readiness_probe: typing.Optional[Probe] = None,
        verbose: bool = False,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        _experimental_enable_snapshot: bool = False,
        client: typing.Optional[modal.client._Client] = None,
        environment_name: typing.Optional[str] = None,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
    ) -> _Sandbox:
        """Create a new Sandbox to run untrusted, arbitrary code.

        The Sandbox's corresponding container will be created asynchronously.

        Args:
            *args: Set the CMD of the Sandbox, overriding any CMD of the container image.
            app: Associate the sandbox with an app. Required unless creating from a container.
            name: Optionally give the sandbox a name. Unique within an app.
            tags: Tags to assign to the Sandbox.
            image: The image to run as the container for the sandbox.
            env: Environment variables to set in the Sandbox.
            secrets: Secrets to inject into the Sandbox as environment variables.
            network_file_systems: Network file systems to mount into the sandbox.
            timeout: Maximum lifetime of the sandbox in seconds.
            idle_timeout: The amount of time in seconds that a sandbox can be idle before being terminated.
            workdir: Working directory of the sandbox.
            gpu: GPU reservation for the sandbox.
            cloud: Cloud provider for the sandbox.
            region: Region or regions to run the sandbox on.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container
                from exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            block_network: Whether to block network access.
            outbound_cidr_allowlist: List of CIDRs the sandbox is allowed to access. If None, all CIDRs are allowed.
            outbound_domain_allowlist: List of domain names the sandbox is allowed to access. Supports
                wildcard prefixes (``*.``); a bare ``"*"`` allows all domains. The outbound policy
                can be replaced later via `Sandbox._experimental_set_outbound_network_policy`.
            inbound_cidr_allowlist:
                List of CIDRs allowed to connect inbound to the sandbox (tunnels and connection tokens). If None,
                all CIDRs are allowed.
            volumes: Mount points for Modal Volumes and CloudBucketMounts.
            pty:
                Enable a PTY for the Sandbox entrypoint command. When enabled, all output (stdout and stderr from the
                process) is multiplexed into stdout, and the stderr stream is effectively empty.
            encrypted_ports: List of ports to tunnel into the sandbox. Encrypted ports are tunneled with TLS.
            h2_ports: List of encrypted ports to tunnel into the sandbox, using HTTP/2.
            unencrypted_ports: List of ports to tunnel into the sandbox without encryption.
            custom_domain:
                Allow connections to the Sandbox via a subdomain of this parent rather than a default Modal domain.
            proxy: Reference to a Modal Proxy to use in front of this Sandbox.
            include_oidc_identity_token:
                If True, the sandbox will receive a MODAL_IDENTITY_TOKEN env var for OIDC-based auth.
            readiness_probe: Probe used to determine when the sandbox has become ready.
            verbose: Enable verbose logging for sandbox operations.
            experimental_options: Experimental options to pass to the sandbox.
            _experimental_enable_snapshot: Enable memory snapshots.
            client: Modal Client to use for the sandbox.
            environment_name: *DEPRECATED* Optionally override the default environment
            pty_info: *DEPRECATED* Use `pty` instead. `pty` will override `pty_info`.
            cidr_allowlist: *DEPRECATED* Use outbound_cidr_allowlist instead.

        Returns:
            A `Sandbox` object representing the created sandbox which can be used to interact with the sandbox.

        Raises:
            AlreadyExistsError: If a sandbox with the same name already exists.

        Examples:
            ```python
            app = modal.App.lookup('sandbox-hello-world', create_if_missing=True)
            sandbox = modal.Sandbox.create("echo", "hello world", app=app)
            print(sandbox.stdout.read())
            sandbox.wait()
            ```
        """
        ...

    @staticmethod
    async def _create(
        *args: str,
        app: typing.Optional[modal.app._App] = None,
        name: typing.Optional[str] = None,
        tags: typing.Optional[dict[str, str]] = None,
        image: typing.Optional[modal._image._Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        mounts: collections.abc.Sequence[modal.mount._Mount] = (),
        network_file_systems: dict[typing.Union[str, os.PathLike], modal.network_file_system._NetworkFileSystem] = {},
        timeout: int = 300,
        idle_timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        gpu: typing.Optional[str] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        block_network: bool = False,
        outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        volumes: dict[
            typing.Union[str, os.PathLike],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        pty: bool = False,
        encrypted_ports: collections.abc.Sequence[int] = [],
        h2_ports: collections.abc.Sequence[int] = [],
        unencrypted_ports: collections.abc.Sequence[int] = [],
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        include_oidc_identity_token: bool = False,
        readiness_probe: typing.Optional[Probe] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        _experimental_enable_snapshot: bool = False,
        client: typing.Optional[modal.client._Client] = None,
        verbose: bool = False,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        custom_domain: typing.Optional[str] = None,
    ):
        """Private method used internally.

        This method exposes some internal arguments (currently `mounts`) which are not in the public API.
        `mounts` is currently only used by modal shell (cli) to provide a function's mounts to the
        sandbox that runs the shell session.
        """
        ...

    @staticmethod
    async def _experimental_create(
        *args: str,
        app: typing.Optional[modal.app._App] = None,
        name: typing.Optional[str] = None,
        tags: typing.Optional[dict[str, str]] = None,
        image: typing.Optional[modal._image._Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        timeout: int = 300,
        idle_timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        block_network: bool = False,
        outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        i6pn: bool = False,
        volumes: dict[
            typing.Union[str, os.PathLike],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        pty: bool = False,
        encrypted_ports: collections.abc.Sequence[int] = [],
        h2_ports: collections.abc.Sequence[int] = [],
        unencrypted_ports: collections.abc.Sequence[int] = [],
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        readiness_probe: typing.Optional[Probe] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        include_oidc_identity_token: bool = False,
        verbose: bool = False,
        custom_domain: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
        _experimental_enable_snapshot: bool = False,
    ) -> _Sandbox:
        """Create a sandbox using the V2 backend.

        Supported features include exec, encrypted tunnels, wait/poll/terminate,
        CPU and memory configuration, region placement, private IPv6 networking
        (i6pn), volumes, cloud bucket mounts (with static credentials via
        `secret=...` or `oidc_auth_role_arn`), OIDC identity tokens, proxies,
        filesystem snapshots, and custom domains (`custom_domain=...` allows
        connections to the sandbox via a subdomain of that parent domain rather
        than a default Modal domain; requires prior setup by Modal).

        `cpu` and `memory` accept either a scalar request or a `(request, limit)`
        tuple that additionally sets a hard limit (fractional CPU cores; memory
        in MiB), matching `Sandbox.create()`.

        Pass `_experimental_enable_snapshot=True` to create a sandbox that can be
        snapshotted with `._experimental_snapshot()`. Features like network
        file systems and GPUs are not supported.

        Set `i6pn=True` to enable private IPv6 networking so sandboxes in the same
        workspace can address each other directly at their `i6pn.modal.local`
        address. i6pn only connects sandboxes co-located on the same routable
        network, so pin every sandbox in the group to the same specific region
        (e.g. `region="us-east-1"`).

        V2 sandboxes created with this method are not currently returned by
        `Sandbox.list()`. A named sandbox can be looked up with
        `Sandbox._experimental_from_name(app_name, name)`; otherwise store
        `sandbox.object_id` and use `Sandbox.from_id(sandbox.object_id)` to
        reattach.
        """
        ...

    def _get_metadata(self) -> modal_proto.api_pb2.SandboxHandleMetadata: ...
    def _hydrate_metadata(self, handle_metadata: typing.Optional[google.protobuf.message.Message]) -> None: ...
    def _hydrate_metadata_v2(self) -> None:
        """Wire up V2 stdio readers that read directly from the worker. Cheap
        to call eagerly: the router connection is opened lazily on first read.
        """
        ...

    def _initialize_from_other(self, other): ...
    def _initialize_from_empty(self): ...
    async def detach(self):
        """Disconnects your client from the sandbox and cleans up resources assoicated with the connection.

        Be sure to only call `detach` when you are done interacting with the sandbox. After calling `detach`,
        any operation using the Sandbox object is not guaranteed to work anymore. If you want to continue interacting
        with a running sandbox, use `Sandbox.from_id` to get a new Sandbox object.
        """
        ...

    @property
    def _client(self) -> modal.client._Client: ...
    @_client.setter
    def _client(self, value): ...
    def _ensure_attached(self): ...
    def _ensure_v1(self, method_name: str): ...
    @staticmethod
    async def from_name(
        app_name: str,
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
    ) -> _Sandbox:
        """Get a running Sandbox by name from a deployed App.

        A Sandbox's name is the `name` argument passed to `Sandbox.create`.

        Args:
            app_name: Name of the deployed app to look up the sandbox under.
            name: Sandbox name to resolve.
            environment_name: Optional environment name for the lookup; defaults to the configured environment.
            client: Modal client to use for the RPC; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Sandbox` handle for the running sandbox.

        Raises:
            NotFoundError: If no running sandbox exists with the given name.
        """
        ...

    @staticmethod
    async def _experimental_from_name(
        app_name: str,
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
    ) -> _Sandbox:
        """Get a running V2 Sandbox by name from a deployed App.

        This looks up V2 sandboxes, ie sandboxes created via `modal.Sandbox._experimental_create`.

        Args:
            app_name: Name of the deployed app to look up the sandbox under.
            name: Sandbox name to resolve.
            environment_name: Optional environment name for the lookup; defaults to the configured environment.
            client: Modal client to use for the RPC; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Sandbox` handle for the running sandbox.

        Raises:
            NotFoundError: If no running sandbox exists with the given name.
        """
        ...

    @staticmethod
    async def from_id(sandbox_id: str, client: typing.Optional[modal.client._Client] = None) -> _Sandbox:
        """Construct a Sandbox from an id and look up the Sandbox result.

        The ID of a Sandbox object can be accessed using `.object_id`.

        Args:
            sandbox_id: Sandbox object ID to attach to.
            client: Modal client to use for the lookup; defaults to the environment client when omitted.

        Returns:
            A `Sandbox` handle with any available result metadata populated from the server.
        """
        ...

    async def get_tags(self) -> dict[str, str]:
        """Fetches any tags (key-value pairs) currently attached to this Sandbox from the server.

        Returns:
            Tags as a map from tag name to tag value.
        """
        ...

    async def set_tags(self, tags: dict[str, str], *, client: typing.Optional[modal.client._Client] = None) -> None:
        """Set tags (key-value pairs) on the Sandbox. Tags can be used to filter results in `Sandbox.list`.

        Setting tags replaces the Sandbox's entire tag set; passing an empty dict clears all tags.

        Args:
            tags: Tag names and values to set on this sandbox.
            client: Deprecated. Prefer setting the client when creating or re-attaching to the sandbox.
        """
        ...

    async def _experimental_set_name(self, name: str) -> None:
        """Assign a name to a running V2 Sandbox that was created without one.

        This is only supported for V2 sandboxes, ie sandboxes created via
        `modal.Sandbox._experimental_create`. A name may only be set once and
        only on a Sandbox that has never had one; afterwards the Sandbox can
        be looked up with `Sandbox._experimental_from_name(app_name, name)`.

        Args:
            name: Name to assign to the Sandbox. Must be unique within the App.

        Raises:
            AlreadyExistsError: If another running Sandbox in the App already holds the name.
            ConflictError: If the Sandbox already has a name or is no longer running.
        """
        ...

    async def _experimental_set_outbound_network_policy(
        self,
        *,
        outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
    ) -> None:
        """Replace the outbound network policy of a running Sandbox.

        Established connections that the new policy no longer permits are
        terminated.

        Args:
            outbound_cidr_allowlist: List of CIDRs the Sandbox is allowed to access. If None, all CIDRs are allowed.
            outbound_domain_allowlist: List of domain names the Sandbox is allowed to access. Supports
                wildcard prefixes (``*.``); a bare ``"*"`` allows all domains.
        """
        ...

    async def _experimental_get_exit_snapshot(self, timeout: typing.Optional[float] = 60) -> modal._image._Image:
        """Get the exit filesystem snapshot image.

        Args:
            timeout: Client-side deadline in seconds (default 60). Use `None` to
                poll until the snapshot reaches a terminal state. Use `0` to
                perform an immediate check.

        Returns:
            The exit snapshot Image.

        Raises:
            InvalidError: If `timeout` is negative, or if exit snapshot is not
                enabled for the sandbox.
            TimeoutError: If `timeout` elapses before the snapshot reaches a
                terminal state. This includes `timeout=0` when the snapshot is
                still pending.
            SnapshotCreationError: If no exit snapshot image will be produced.
            NotFoundError: If the sandbox does not exist.
            PermissionDeniedError: If the caller cannot access the sandbox.
            InternalError: If persisted snapshot state is malformed.
            ServiceError: If a transient client/server communication failure occurs.
        """
        ...

    async def snapshot_filesystem(
        self, timeout: int = 55, *, ttl: typing.Optional[int] = 2592000
    ) -> modal._image._Image:
        """Snapshot the filesystem of the Sandbox.

        Args:
            timeout:
                Maximum time in seconds to wait for the snapshot operation. If the snapshot does not return within
                that window, the call is cancelled and `modal.exception.TimeoutError` is raised.
            ttl:
                The resulting Image is retained for `ttl` seconds (default: 30 days). Pass `ttl=None` to retain
                the image indefinitely.

        Returns:
            An [`Image`](https://modal.com/docs/sdk/py/latest/Image) object which can be used to spawn a new
            Sandbox with the same filesystem.
        """
        ...

    async def _legacy_snapshot_filesystem(self, timeout: int = 55) -> modal._image._Image: ...
    async def mount_image(
        self,
        path: typing.Union[pathlib.PurePosixPath, str],
        image: modal._image._Image,
        *,
        _experimental_encryption_key: typing.Optional[bytes] = None,
    ):
        """Mount an Image at a specified path in a running Sandbox.

        `path` should be a directory that is **not** the root path (`/`). If the path doesn't exist
        it will be created. If it exists and contains data, the previous directory will be replaced
        by the mount.

        The `image` argument supports any Image that has an object ID, including:
        - Images built using `image.build()`
        - Images referenced by ID, e.g. `Image.from_id(...)`
        - Filesystem/directory snapshots, e.g. created by `.snapshot_directory()` or `.snapshot_filesystem()`
        - Empty images created with `Image.from_scratch()`

        Args:
            path: Absolute mount point directory inside the sandbox (not `/`).
            image: Image to mount at `path` (must be built, referenced by ID, or snapshot-based as described above).


        Examples:
            ```py notest
            user_project_snapshot: Image = sandbox_session_1.snapshot_directory("/user_project")

            # You can later mount this snapshot to another Sandbox:
            sandbox_session_2 = modal.Sandbox.create(...)
            sandbox_session_2.mount_image("/user_project", user_project_snapshot)
            sandbox_session_2.filesystem.list_files("/user_project")
            ```
        """
        ...

    async def unmount_image(self, path: typing.Union[pathlib.PurePosixPath, str]):
        """Unmount a previously mounted Image from a running Sandbox.

        `path` must be the exact mount point that was passed to `.mount_image()`.
        After unmounting, the underlying Sandbox filesystem at that path becomes
        visible again.

        Args:
            path: Absolute mount point directory to unmount.
        """
        ...

    async def snapshot_directory(
        self,
        path: typing.Union[pathlib.PurePosixPath, str],
        *,
        timeout: int = 55,
        ttl: typing.Optional[int] = 2592000,
        _experimental_encryption_key: typing.Optional[bytes] = None,
    ) -> modal._image._Image:
        """Snapshot a directory in a running Sandbox, creating a new Image with its content.

        `timeout` If the snapshot does not return within that window, the call is cancelled
        and `modal.exception.TimeoutError` is raised.

        `ttl` The resulting Image is retained for `ttl` seconds (default: 30 days)
        Pass `ttl=None` to retain the image indefinitely.

        Args:
            path: Absolute path of the directory inside the sandbox to snapshot.

        Returns:
            An `Image` containing the directory contents.

        Examples:
            ```py notest
            user_project_snapshot: Image = sandbox_session_1.snapshot_directory("/user_project")

            # You can later mount this snapshot to another Sandbox:
            sandbox_session_2 = modal.Sandbox.create(...)
            sandbox_session_2.mount_image("/user_project", user_project_snapshot)
            sandbox_session_2.filesystem.list_files("/user_project")
            ```
        """
        ...

    async def wait(self, raise_on_termination: bool = True):
        """Wait for the Sandbox to finish running.

        Args:
            raise_on_termination: If True, raise when the sandbox is terminated externally.
        """
        ...

    async def wait_until_ready(self, *, timeout: int = 300) -> None:
        """Wait for the Sandbox readiness probe to report that the Sandbox is ready.

        The Sandbox must be configured with a `readiness_probe` in order to use this method.

        Args:
            timeout: Maximum time in seconds to wait for readiness.


        Examples:
            ```py notest
            app = modal.App.lookup('sandbox-wait-until-ready', create_if_missing=True)
            sandbox = modal.Sandbox.create(
                "python3", "-m", "http.server", "8080",
                readiness_probe=modal.Probe.with_tcp(8080),
                app=app,
            )
            sandbox.wait_until_ready()
            ```
        """
        ...

    async def tunnels(self, timeout: int = 50) -> dict[int, modal._tunnel.Tunnel]:
        """Get Tunnel metadata for the sandbox.

        NOTE: Previous to client [v0.64.153](https://modal.com/docs/sdk/py/changelog#064153-2024-09-30), this
        returned a list of `TunnelData` objects.

        Args:
            timeout: Maximum time in seconds to wait for tunnel metadata when not already cached.

        Returns:
            A dictionary mapping container port to `Tunnel` metadata.

        Raises:
            SandboxTimeoutError: If the tunnels are not available after the timeout.
        """
        ...

    async def create_connect_token(
        self, user_metadata: typing.Union[str, dict[str, typing.Any], None] = None, port: int = 8080
    ) -> modal.types.SandboxConnectCredentials:
        """Create a token for making HTTP connections to the Sandbox.

        Accepts an optional user_metadata string or dict to associate with the token. This metadata
        will be added to the headers by the proxy when forwarding requests to the Sandbox.
        Also accepts a port that requests will be routed to.

        Args:
            user_metadata: Optional JSON-serializable metadata or string stored with the connect token.
            port: Optional container port that requests are routed to when using this token.

        Returns:
            URL and token credentials for connecting to the sandbox over HTTP.
        """
        ...

    async def reload_volumes(self, *, timeout: int = 55) -> None:
        """Reload all Volumes mounted in the Sandbox.

        Added in v1.1.0.

        Blocks until the Volumes have been reloaded, bounded by `timeout` (55 seconds by default). If the reload
        does not complete within that window, `modal.exception.TimeoutError` is raised; note that the reload may
        still complete in the background.

        Args:
            timeout: Maximum time in seconds to wait for the reload. Must be positive.
        """
        ...

    @typing.overload
    async def terminate(self, *, wait: typing.Literal[True]) -> int: ...
    @typing.overload
    async def terminate(self, *, wait: typing.Literal[False] = False) -> None: ...
    async def poll(self) -> typing.Optional[int]:
        """Check if the Sandbox has finished running.

        Returns:
            `None` if the Sandbox is still running, otherwise the exit code.
        """
        ...

    async def _get_task_id(self, raise_if_task_complete=False) -> str: ...
    async def _get_command_router_client(
        self, task_id: str
    ) -> modal._utils.task_command_router_client.TaskCommandRouterClient: ...
    @property
    def _experimental_sidecars(self) -> _SidecarManager:
        """Manage sidecar containers running in this Sandbox."""
        ...

    @typing.overload
    async def exec(
        self,
        *args: str,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        text: typing.Literal[True] = True,
        bufsize: typing.Literal[-1, 1] = -1,
        pty: bool = False,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        _pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
    ) -> modal.container_process._ContainerProcess[str]: ...
    @typing.overload
    async def exec(
        self,
        *args: str,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        text: typing.Literal[False] = False,
        bufsize: typing.Literal[-1, 1] = -1,
        pty: bool = False,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        _pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
    ) -> modal.container_process._ContainerProcess[bytes]: ...
    async def _exec(
        self,
        *args: str,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        text: bool = True,
        bufsize: typing.Literal[-1, 1] = -1,
        container_id: typing.Optional[str] = None,
    ) -> typing.Union[modal.container_process._ContainerProcess[bytes], modal.container_process._ContainerProcess[str]]:
        """Private method used internally.

        This method exposes some internal arguments (currently `pty_info`) which are not in the public API.
        """
        ...

    async def _exec_through_command_router(
        self,
        *args: str,
        task_id: str,
        command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        secret_ids: typing.Optional[collections.abc.Collection[str]] = None,
        env: typing.Optional[dict[str, str]] = None,
        text: bool = True,
        bufsize: typing.Literal[-1, 1] = -1,
        runtime_debug: bool = False,
        container_id: typing.Optional[str] = None,
    ) -> typing.Union[modal.container_process._ContainerProcess[bytes], modal.container_process._ContainerProcess[str]]:
        """Execute a command through a task command router running on the Modal worker."""
        ...

    async def _experimental_snapshot(self) -> modal.snapshot._SandboxSnapshot: ...
    @staticmethod
    async def _experimental_from_snapshot(
        snapshot: modal.snapshot._SandboxSnapshot,
        client: typing.Optional[modal.client._Client] = None,
        *,
        name: typing.Optional[str] = _DEFAULT_SANDBOX_NAME_OVERRIDE,
    ):
        """Restore a Sandbox from a memory snapshot.

        The restore targets the same backend the snapshot was taken from. A V1
        snapshot restores as a V1 sandbox, a V2 snapshot as a V2 sandbox.
        """
        ...

    @staticmethod
    async def _experimental_from_snapshot_v2(
        snapshot: modal.snapshot._SandboxSnapshot,
        client: modal.client._Client,
        *,
        override_type: int,
        name_override: typing.Optional[str],
    ) -> _Sandbox: ...
    @property
    def filesystem(self) -> modal.sandbox_fs._SandboxFilesystem:
        """Namespace for Sandbox filesystem APIs."""
        ...

    @typing.overload
    async def open(self, path: str) -> modal.file_io._FileIO[str]: ...
    @typing.overload
    async def open(self, path: str, mode: _typeshed.OpenTextMode) -> modal.file_io._FileIO[str]: ...
    @typing.overload
    async def open(self, path: str, mode: _typeshed.OpenBinaryMode) -> modal.file_io._FileIO[bytes]: ...
    async def ls(self, path: str) -> list[str]:
        """[Alpha] List the contents of a directory in the Sandbox.

        **Deprecated (2026-04-15):** Use `Sandbox.filesystem.list_files()` instead for improved reliability.

        Args:
            path: Absolute directory path inside the sandbox.

        Returns:
            Entry names in the directory as a list of strings.
        """
        ...

    async def mkdir(self, path: str, parents: bool = False) -> None:
        """[Alpha] Create a new directory in the Sandbox.

        **Deprecated (2026-04-15):** Use `Sandbox.filesystem.make_directory()` instead for improved reliability.
        """
        ...

    async def rm(self, path: str, recursive: bool = False) -> None:
        """[Alpha] Remove a file or directory in the Sandbox.

        **Deprecated (2026-04-15):** Use `Sandbox.filesystem.remove()` instead for improved reliability.
        """
        ...

    def watch(
        self,
        path: str,
        filter: typing.Optional[list[modal.types.FileWatchEventType]] = None,
        recursive: typing.Optional[bool] = None,
        timeout: typing.Optional[int] = None,
    ) -> collections.abc.AsyncIterator[modal.types.FileWatchEvent]:
        """[Alpha] Watch a file or directory in the Sandbox for changes.

        **Deprecated (2026-05-08):** Use `Sandbox.filesystem.watch()` instead for improved reliability.

        Args:
            path: Absolute path to watch.
            filter: Optional list of event types to include.
            recursive: Whether to watch subdirectories; None uses server defaults.
            timeout: Optional timeout for the watch stream.

        Returns:
            An async iterator of `FileWatchEvent` values.
        """
        ...

    @property
    def stdout(self) -> modal.io_streams._StreamReader[str]:
        """[`StreamReader`](https://modal.com/docs/sdk/py/latest/io_streams#streamreader)
        for the sandbox's stdout stream.

        Returns:
            Stream reader for sandbox stdout.
        """
        ...

    @property
    def stderr(self) -> modal.io_streams._StreamReader[str]:
        """[`StreamReader`](https://modal.com/docs/sdk/py/latest/io_streams#streamreader)
        for the Sandbox's stderr stream.

        Returns:
            Stream reader for sandbox stderr.
        """
        ...

    @property
    def stdin(self) -> modal.io_streams._StreamWriter:
        """[`StreamWriter`](https://modal.com/docs/sdk/py/latest/io_streams#streamwriter)
        for the Sandbox's stdin stream.

        Returns:
            Stream writer for sandbox stdin.
        """
        ...

    @property
    def returncode(self) -> typing.Optional[int]:
        """Return code of the Sandbox process if it has finished running, else `None`.

        Returns:
            Exit code when the sandbox process has completed, otherwise None.
        """
        ...

    @staticmethod
    def list(
        *,
        app_id: typing.Optional[str] = None,
        tags: typing.Optional[dict[str, str]] = None,
        client: typing.Optional[modal.client._Client] = None,
    ) -> collections.abc.AsyncGenerator[_Sandbox, None]:
        """List all Sandboxes for the current Environment or App ID (if specified). If tags are specified, only
        Sandboxes that have at least those tags are returned.

        Args:
            app_id: If set, restrict results to sandboxes under this app ID.
            tags: If set, only sandboxes containing at least these tags are returned.
            client: Modal client to use for listing; defaults to `Client.from_env()` when omitted.

        Returns:
            An async generator yielding `Sandbox` objects.
        """
        ...

    @staticmethod
    def _experimental_list(
        *,
        app_id: typing.Optional[str] = None,
        tags: typing.Optional[dict[str, str]] = None,
        client: typing.Optional[modal.client._Client] = None,
    ) -> collections.abc.AsyncGenerator[_Sandbox, None]:
        """List v2 Sandboxes in an App.

        This function lists v2 sandboxes, ie sandboxes created via modal.Sandbox._experimental_create.

        Args:
            app_id: The App to list Sandboxes under.
            tags: If set, only sandboxes containing at least these tags are returned.
            client: Optional client to use for the session.

        Yields:
            `Sandbox` objects that are currently running in the App.
        """
        ...

class _SidecarContainer:
    """Handle to an additional container running in a Sandbox."""

    _result: typing.Optional[modal_proto.api_pb2.GenericResult]
    _filesystem: typing.Optional[modal.sandbox_fs._SandboxFilesystem]

    def __init__(
        self,
        sandbox: _Sandbox,
        container_id: str,
        container_name: str,
        result: typing.Optional[modal_proto.api_pb2.GenericResult] = None,
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    @property
    def object_id(self) -> str: ...
    @property
    def name(self) -> str: ...
    @staticmethod
    def _from_container_info(
        sandbox: _Sandbox, container_info: modal_proto.task_command_router_pb2.TaskContainerInfo
    ) -> _SidecarContainer: ...
    async def _get_command_router(self) -> tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]:
        """Get task ID and command router client."""
        ...

    @typing.overload
    async def exec(
        self,
        *args: str,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        text: typing.Literal[True] = True,
        bufsize: typing.Literal[-1, 1] = -1,
        pty: bool = False,
    ) -> modal.container_process._ContainerProcess[str]: ...
    @typing.overload
    async def exec(
        self,
        *args: str,
        stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
        timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        text: typing.Literal[False],
        bufsize: typing.Literal[-1, 1] = -1,
        pty: bool = False,
    ) -> modal.container_process._ContainerProcess[bytes]: ...
    @property
    def filesystem(self) -> modal.sandbox_fs._SandboxFilesystem:
        """Namespace for Sandbox filesystem APIs."""
        ...

    async def wait(self, raise_on_termination: bool = True) -> None: ...
    async def poll(self) -> typing.Optional[int]: ...
    @typing.overload
    async def terminate(self, *, wait: typing.Literal[True]) -> int: ...
    @typing.overload
    async def terminate(self, *, wait: typing.Literal[False] = False) -> None: ...

class _SidecarManager:
    """Creates and manages sidecar containers in a Sandbox."""
    def __init__(self, sandbox: _Sandbox) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    async def _get_command_router(self) -> tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]:
        """Get task ID and command router client."""
        ...

    async def create(
        self,
        *args: str,
        name: str,
        image: modal._image._Image,
        env: typing.Optional[dict[str, str]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        workdir: typing.Optional[str] = None,
        volumes: typing.Optional[dict[typing.Union[str, os.PathLike], modal.volume._Volume]] = None,
    ) -> _SidecarContainer:
        """Create a sidecar container running alongside the Sandbox's main container.

        Sidecar containers share the Sandbox's lifecycle but run their own Image and command. They
        can be used to run auxiliary processes, such as a database or a service the main container
        depends on.

        Args:
            *args: Command and arguments to run inside the sidecar container.
            name: Unique name for the sidecar container. The name ``"main"`` is reserved.
            image: Image to run the sidecar container with. Must be a pre-built or referenced Image.
            env: Environment variables to set in the sidecar container.
            secrets: Secrets to inject as environment variables in the sidecar container.
            workdir: Working directory for the command; must be absolute if set.
            volumes: Mapping of mount paths to `Volume` objects to mount in the sidecar container.

        Returns:
            A `SidecarContainer` handle for the running container.
        """
        ...

    async def get(self, *, name: str, include_terminated: bool = False) -> _SidecarContainer: ...
    async def list(self, include_terminated: bool = False) -> list[_SidecarContainer]: ...

class SidecarContainer:
    """Handle to an additional container running in a Sandbox."""

    _result: typing.Optional[modal_proto.api_pb2.GenericResult]
    _filesystem: typing.Optional[modal.sandbox_fs.SandboxFilesystem]

    def __init__(
        self,
        sandbox: Sandbox,
        container_id: str,
        container_name: str,
        result: typing.Optional[modal_proto.api_pb2.GenericResult] = None,
    ) -> None: ...
    @property
    def object_id(self) -> str: ...
    @property
    def name(self) -> str: ...
    @staticmethod
    def _from_container_info(
        sandbox: Sandbox, container_info: modal_proto.task_command_router_pb2.TaskContainerInfo
    ) -> SidecarContainer: ...

    class ___get_command_router_spec(typing_extensions.Protocol):
        def __call__(self, /) -> tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]:
            """Get task ID and command router client."""
            ...

        async def aio(self, /) -> tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]:
            """Get task ID and command router client."""
            ...

    _get_command_router: ___get_command_router_spec

    class __exec_spec(typing_extensions.Protocol):
        @typing.overload
        def __call__(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[True] = True,
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
        ) -> modal.container_process.ContainerProcess[str]: ...
        @typing.overload
        def __call__(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[False],
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
        ) -> modal.container_process.ContainerProcess[bytes]: ...
        @typing.overload
        async def aio(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[True] = True,
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
        ) -> modal.container_process.ContainerProcess[str]: ...
        @typing.overload
        async def aio(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[False],
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
        ) -> modal.container_process.ContainerProcess[bytes]: ...

    exec: __exec_spec

    @property
    def filesystem(self) -> modal.sandbox_fs.SandboxFilesystem:
        """Namespace for Sandbox filesystem APIs."""
        ...

    class __wait_spec(typing_extensions.Protocol):
        def __call__(self, /, raise_on_termination: bool = True) -> None: ...
        async def aio(self, /, raise_on_termination: bool = True) -> None: ...

    wait: __wait_spec

    class __poll_spec(typing_extensions.Protocol):
        def __call__(self, /) -> typing.Optional[int]: ...
        async def aio(self, /) -> typing.Optional[int]: ...

    poll: __poll_spec

    class __terminate_spec(typing_extensions.Protocol):
        @typing.overload
        def __call__(self, /, *, wait: typing.Literal[True]) -> int: ...
        @typing.overload
        def __call__(self, /, *, wait: typing.Literal[False] = False) -> None: ...
        @typing.overload
        async def aio(self, /, *, wait: typing.Literal[True]) -> int: ...
        @typing.overload
        async def aio(self, /, *, wait: typing.Literal[False] = False) -> None: ...

    terminate: __terminate_spec

class SidecarManager:
    """Creates and manages sidecar containers in a Sandbox."""
    def __init__(self, sandbox: Sandbox) -> None: ...

    class ___get_command_router_spec(typing_extensions.Protocol):
        def __call__(self, /) -> tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]:
            """Get task ID and command router client."""
            ...

        async def aio(self, /) -> tuple[str, modal._utils.task_command_router_client.TaskCommandRouterClient]:
            """Get task ID and command router client."""
            ...

    _get_command_router: ___get_command_router_spec

    class __create_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *args: str,
            name: str,
            image: modal.image.Image,
            env: typing.Optional[dict[str, str]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            workdir: typing.Optional[str] = None,
            volumes: typing.Optional[dict[typing.Union[str, os.PathLike], modal.volume.Volume]] = None,
        ) -> SidecarContainer:
            """Create a sidecar container running alongside the Sandbox's main container.

            Sidecar containers share the Sandbox's lifecycle but run their own Image and command. They
            can be used to run auxiliary processes, such as a database or a service the main container
            depends on.

            Args:
                *args: Command and arguments to run inside the sidecar container.
                name: Unique name for the sidecar container. The name ``"main"`` is reserved.
                image: Image to run the sidecar container with. Must be a pre-built or referenced Image.
                env: Environment variables to set in the sidecar container.
                secrets: Secrets to inject as environment variables in the sidecar container.
                workdir: Working directory for the command; must be absolute if set.
                volumes: Mapping of mount paths to `Volume` objects to mount in the sidecar container.

            Returns:
                A `SidecarContainer` handle for the running container.
            """
            ...

        async def aio(
            self,
            /,
            *args: str,
            name: str,
            image: modal.image.Image,
            env: typing.Optional[dict[str, str]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            workdir: typing.Optional[str] = None,
            volumes: typing.Optional[dict[typing.Union[str, os.PathLike], modal.volume.Volume]] = None,
        ) -> SidecarContainer:
            """Create a sidecar container running alongside the Sandbox's main container.

            Sidecar containers share the Sandbox's lifecycle but run their own Image and command. They
            can be used to run auxiliary processes, such as a database or a service the main container
            depends on.

            Args:
                *args: Command and arguments to run inside the sidecar container.
                name: Unique name for the sidecar container. The name ``"main"`` is reserved.
                image: Image to run the sidecar container with. Must be a pre-built or referenced Image.
                env: Environment variables to set in the sidecar container.
                secrets: Secrets to inject as environment variables in the sidecar container.
                workdir: Working directory for the command; must be absolute if set.
                volumes: Mapping of mount paths to `Volume` objects to mount in the sidecar container.

            Returns:
                A `SidecarContainer` handle for the running container.
            """
            ...

    create: __create_spec

    class __get_spec(typing_extensions.Protocol):
        def __call__(self, /, *, name: str, include_terminated: bool = False) -> SidecarContainer: ...
        async def aio(self, /, *, name: str, include_terminated: bool = False) -> SidecarContainer: ...

    get: __get_spec

    class __list_spec(typing_extensions.Protocol):
        def __call__(self, /, include_terminated: bool = False) -> list[SidecarContainer]: ...
        async def aio(self, /, include_terminated: bool = False) -> list[SidecarContainer]: ...

    list: __list_spec

class Sandbox(modal.object.Object):
    """A `Sandbox` object lets you interact with a running sandbox. This API is similar to Python's
    [asyncio.subprocess.Process](https://docs.python.org/3/library/asyncio-subprocess.html#asyncio.subprocess.Process).

    Refer to the [guide](https://modal.com/docs/guide/sandbox) on how to spawn and use sandboxes.
    """

    _result: typing.Optional[modal_proto.api_pb2.GenericResult]
    _stdout: modal.io_streams.StreamReader[str]
    _stderr: modal.io_streams.StreamReader[str]
    _stdin: modal.io_streams.StreamWriter
    _task_id: typing.Optional[str]
    _tunnels: typing.Optional[dict[int, modal._tunnel.Tunnel]]
    _enable_snapshot: bool
    _command_router_client: typing.Optional[modal._utils.task_command_router_client.TaskCommandRouterClient]
    _attached: bool
    _filesystem: typing.Optional[modal.sandbox_fs.SandboxFilesystem]
    _is_v2: bool
    _app_id: typing.Optional[str]

    def __init__(self, *args, **kwargs):
        """mdmd:hidden"""
        ...

    @staticmethod
    def _default_pty_info() -> modal_proto.api_pb2.PTYInfo: ...
    @staticmethod
    def _new(
        args: collections.abc.Sequence[str],
        image: modal.image.Image,
        secrets: collections.abc.Collection[modal.secret.Secret],
        name: typing.Optional[str] = None,
        timeout: int = 300,
        idle_timeout: typing.Optional[int] = None,
        workdir: typing.Optional[str] = None,
        gpu: typing.Optional[str] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        mounts: collections.abc.Sequence[modal.mount.Mount] = (),
        network_file_systems: dict[typing.Union[str, os.PathLike], modal.network_file_system.NetworkFileSystem] = {},
        block_network: bool = False,
        outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        volumes: dict[
            typing.Union[str, os.PathLike], typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount]
        ] = {},
        pty: bool = False,
        pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        encrypted_ports: collections.abc.Sequence[int] = [],
        h2_ports: collections.abc.Sequence[int] = [],
        unencrypted_ports: collections.abc.Sequence[int] = [],
        proxy: typing.Optional[modal.proxy.Proxy] = None,
        readiness_probe: typing.Optional[Probe] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        tags: typing.Optional[dict[str, str]] = None,
        enable_snapshot: bool = False,
        verbose: bool = False,
        custom_domain: typing.Optional[str] = None,
        include_oidc_identity_token: bool = False,
    ) -> Sandbox:
        """mdmd:hidden"""
        ...

    class __create_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *args: str,
            app: typing.Optional[modal.app.App] = None,
            name: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            image: typing.Optional[modal.image.Image] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            network_file_systems: dict[
                typing.Union[str, os.PathLike], modal.network_file_system.NetworkFileSystem
            ] = {},
            timeout: int = 300,
            idle_timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            gpu: typing.Optional[str] = None,
            cloud: typing.Optional[str] = None,
            region: typing.Union[str, collections.abc.Sequence[str], None] = None,
            cpu: typing.Union[float, tuple[float, float], None] = None,
            memory: typing.Union[int, tuple[int, int], None] = None,
            block_network: bool = False,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            volumes: dict[
                typing.Union[str, os.PathLike],
                typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
            ] = {},
            pty: bool = False,
            encrypted_ports: collections.abc.Sequence[int] = [],
            h2_ports: collections.abc.Sequence[int] = [],
            unencrypted_ports: collections.abc.Sequence[int] = [],
            custom_domain: typing.Optional[str] = None,
            proxy: typing.Optional[modal.proxy.Proxy] = None,
            include_oidc_identity_token: bool = False,
            readiness_probe: typing.Optional[Probe] = None,
            verbose: bool = False,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
            _experimental_enable_snapshot: bool = False,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        ) -> Sandbox:
            """Create a new Sandbox to run untrusted, arbitrary code.

            The Sandbox's corresponding container will be created asynchronously.

            Args:
                *args: Set the CMD of the Sandbox, overriding any CMD of the container image.
                app: Associate the sandbox with an app. Required unless creating from a container.
                name: Optionally give the sandbox a name. Unique within an app.
                tags: Tags to assign to the Sandbox.
                image: The image to run as the container for the sandbox.
                env: Environment variables to set in the Sandbox.
                secrets: Secrets to inject into the Sandbox as environment variables.
                network_file_systems: Network file systems to mount into the sandbox.
                timeout: Maximum lifetime of the sandbox in seconds.
                idle_timeout: The amount of time in seconds that a sandbox can be idle before being terminated.
                workdir: Working directory of the sandbox.
                gpu: GPU reservation for the sandbox.
                cloud: Cloud provider for the sandbox.
                region: Region or regions to run the sandbox on.
                cpu:
                    Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                    additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container
                    from exceeding its specified limit.
                memory:
                    Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                    additionally specify a hard limit in MiB.
                block_network: Whether to block network access.
                outbound_cidr_allowlist: List of CIDRs the sandbox is allowed to access. If None, all CIDRs are allowed.
                outbound_domain_allowlist: List of domain names the sandbox is allowed to access. Supports
                    wildcard prefixes (``*.``); a bare ``"*"`` allows all domains. The outbound policy
                    can be replaced later via `Sandbox._experimental_set_outbound_network_policy`.
                inbound_cidr_allowlist:
                    List of CIDRs allowed to connect inbound to the sandbox (tunnels and connection tokens). If None,
                    all CIDRs are allowed.
                volumes: Mount points for Modal Volumes and CloudBucketMounts.
                pty:
                    Enable a PTY for the Sandbox entrypoint command. When enabled, all output (stdout and stderr from the
                    process) is multiplexed into stdout, and the stderr stream is effectively empty.
                encrypted_ports: List of ports to tunnel into the sandbox. Encrypted ports are tunneled with TLS.
                h2_ports: List of encrypted ports to tunnel into the sandbox, using HTTP/2.
                unencrypted_ports: List of ports to tunnel into the sandbox without encryption.
                custom_domain:
                    Allow connections to the Sandbox via a subdomain of this parent rather than a default Modal domain.
                proxy: Reference to a Modal Proxy to use in front of this Sandbox.
                include_oidc_identity_token:
                    If True, the sandbox will receive a MODAL_IDENTITY_TOKEN env var for OIDC-based auth.
                readiness_probe: Probe used to determine when the sandbox has become ready.
                verbose: Enable verbose logging for sandbox operations.
                experimental_options: Experimental options to pass to the sandbox.
                _experimental_enable_snapshot: Enable memory snapshots.
                client: Modal Client to use for the sandbox.
                environment_name: *DEPRECATED* Optionally override the default environment
                pty_info: *DEPRECATED* Use `pty` instead. `pty` will override `pty_info`.
                cidr_allowlist: *DEPRECATED* Use outbound_cidr_allowlist instead.

            Returns:
                A `Sandbox` object representing the created sandbox which can be used to interact with the sandbox.

            Raises:
                AlreadyExistsError: If a sandbox with the same name already exists.

            Examples:
                ```python
                app = modal.App.lookup('sandbox-hello-world', create_if_missing=True)
                sandbox = modal.Sandbox.create("echo", "hello world", app=app)
                print(sandbox.stdout.read())
                sandbox.wait()
                ```
            """
            ...

        async def aio(
            self,
            /,
            *args: str,
            app: typing.Optional[modal.app.App] = None,
            name: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            image: typing.Optional[modal.image.Image] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            network_file_systems: dict[
                typing.Union[str, os.PathLike], modal.network_file_system.NetworkFileSystem
            ] = {},
            timeout: int = 300,
            idle_timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            gpu: typing.Optional[str] = None,
            cloud: typing.Optional[str] = None,
            region: typing.Union[str, collections.abc.Sequence[str], None] = None,
            cpu: typing.Union[float, tuple[float, float], None] = None,
            memory: typing.Union[int, tuple[int, int], None] = None,
            block_network: bool = False,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            volumes: dict[
                typing.Union[str, os.PathLike],
                typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
            ] = {},
            pty: bool = False,
            encrypted_ports: collections.abc.Sequence[int] = [],
            h2_ports: collections.abc.Sequence[int] = [],
            unencrypted_ports: collections.abc.Sequence[int] = [],
            custom_domain: typing.Optional[str] = None,
            proxy: typing.Optional[modal.proxy.Proxy] = None,
            include_oidc_identity_token: bool = False,
            readiness_probe: typing.Optional[Probe] = None,
            verbose: bool = False,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
            _experimental_enable_snapshot: bool = False,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        ) -> Sandbox:
            """Create a new Sandbox to run untrusted, arbitrary code.

            The Sandbox's corresponding container will be created asynchronously.

            Args:
                *args: Set the CMD of the Sandbox, overriding any CMD of the container image.
                app: Associate the sandbox with an app. Required unless creating from a container.
                name: Optionally give the sandbox a name. Unique within an app.
                tags: Tags to assign to the Sandbox.
                image: The image to run as the container for the sandbox.
                env: Environment variables to set in the Sandbox.
                secrets: Secrets to inject into the Sandbox as environment variables.
                network_file_systems: Network file systems to mount into the sandbox.
                timeout: Maximum lifetime of the sandbox in seconds.
                idle_timeout: The amount of time in seconds that a sandbox can be idle before being terminated.
                workdir: Working directory of the sandbox.
                gpu: GPU reservation for the sandbox.
                cloud: Cloud provider for the sandbox.
                region: Region or regions to run the sandbox on.
                cpu:
                    Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                    additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container
                    from exceeding its specified limit.
                memory:
                    Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                    additionally specify a hard limit in MiB.
                block_network: Whether to block network access.
                outbound_cidr_allowlist: List of CIDRs the sandbox is allowed to access. If None, all CIDRs are allowed.
                outbound_domain_allowlist: List of domain names the sandbox is allowed to access. Supports
                    wildcard prefixes (``*.``); a bare ``"*"`` allows all domains. The outbound policy
                    can be replaced later via `Sandbox._experimental_set_outbound_network_policy`.
                inbound_cidr_allowlist:
                    List of CIDRs allowed to connect inbound to the sandbox (tunnels and connection tokens). If None,
                    all CIDRs are allowed.
                volumes: Mount points for Modal Volumes and CloudBucketMounts.
                pty:
                    Enable a PTY for the Sandbox entrypoint command. When enabled, all output (stdout and stderr from the
                    process) is multiplexed into stdout, and the stderr stream is effectively empty.
                encrypted_ports: List of ports to tunnel into the sandbox. Encrypted ports are tunneled with TLS.
                h2_ports: List of encrypted ports to tunnel into the sandbox, using HTTP/2.
                unencrypted_ports: List of ports to tunnel into the sandbox without encryption.
                custom_domain:
                    Allow connections to the Sandbox via a subdomain of this parent rather than a default Modal domain.
                proxy: Reference to a Modal Proxy to use in front of this Sandbox.
                include_oidc_identity_token:
                    If True, the sandbox will receive a MODAL_IDENTITY_TOKEN env var for OIDC-based auth.
                readiness_probe: Probe used to determine when the sandbox has become ready.
                verbose: Enable verbose logging for sandbox operations.
                experimental_options: Experimental options to pass to the sandbox.
                _experimental_enable_snapshot: Enable memory snapshots.
                client: Modal Client to use for the sandbox.
                environment_name: *DEPRECATED* Optionally override the default environment
                pty_info: *DEPRECATED* Use `pty` instead. `pty` will override `pty_info`.
                cidr_allowlist: *DEPRECATED* Use outbound_cidr_allowlist instead.

            Returns:
                A `Sandbox` object representing the created sandbox which can be used to interact with the sandbox.

            Raises:
                AlreadyExistsError: If a sandbox with the same name already exists.

            Examples:
                ```python
                app = modal.App.lookup('sandbox-hello-world', create_if_missing=True)
                sandbox = modal.Sandbox.create("echo", "hello world", app=app)
                print(sandbox.stdout.read())
                sandbox.wait()
                ```
            """
            ...

    create: typing.ClassVar[__create_spec]

    class ___create_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *args: str,
            app: typing.Optional[modal.app.App] = None,
            name: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            image: typing.Optional[modal.image.Image] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            mounts: collections.abc.Sequence[modal.mount.Mount] = (),
            network_file_systems: dict[
                typing.Union[str, os.PathLike], modal.network_file_system.NetworkFileSystem
            ] = {},
            timeout: int = 300,
            idle_timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            gpu: typing.Optional[str] = None,
            cloud: typing.Optional[str] = None,
            region: typing.Union[str, collections.abc.Sequence[str], None] = None,
            cpu: typing.Union[float, tuple[float, float], None] = None,
            memory: typing.Union[int, tuple[int, int], None] = None,
            block_network: bool = False,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            volumes: dict[
                typing.Union[str, os.PathLike],
                typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
            ] = {},
            pty: bool = False,
            encrypted_ports: collections.abc.Sequence[int] = [],
            h2_ports: collections.abc.Sequence[int] = [],
            unencrypted_ports: collections.abc.Sequence[int] = [],
            proxy: typing.Optional[modal.proxy.Proxy] = None,
            include_oidc_identity_token: bool = False,
            readiness_probe: typing.Optional[Probe] = None,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
            _experimental_enable_snapshot: bool = False,
            client: typing.Optional[modal.client.Client] = None,
            verbose: bool = False,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            custom_domain: typing.Optional[str] = None,
        ):
            """Private method used internally.

            This method exposes some internal arguments (currently `mounts`) which are not in the public API.
            `mounts` is currently only used by modal shell (cli) to provide a function's mounts to the
            sandbox that runs the shell session.
            """
            ...

        async def aio(
            self,
            /,
            *args: str,
            app: typing.Optional[modal.app.App] = None,
            name: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            image: typing.Optional[modal.image.Image] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            mounts: collections.abc.Sequence[modal.mount.Mount] = (),
            network_file_systems: dict[
                typing.Union[str, os.PathLike], modal.network_file_system.NetworkFileSystem
            ] = {},
            timeout: int = 300,
            idle_timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            gpu: typing.Optional[str] = None,
            cloud: typing.Optional[str] = None,
            region: typing.Union[str, collections.abc.Sequence[str], None] = None,
            cpu: typing.Union[float, tuple[float, float], None] = None,
            memory: typing.Union[int, tuple[int, int], None] = None,
            block_network: bool = False,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            volumes: dict[
                typing.Union[str, os.PathLike],
                typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
            ] = {},
            pty: bool = False,
            encrypted_ports: collections.abc.Sequence[int] = [],
            h2_ports: collections.abc.Sequence[int] = [],
            unencrypted_ports: collections.abc.Sequence[int] = [],
            proxy: typing.Optional[modal.proxy.Proxy] = None,
            include_oidc_identity_token: bool = False,
            readiness_probe: typing.Optional[Probe] = None,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
            _experimental_enable_snapshot: bool = False,
            client: typing.Optional[modal.client.Client] = None,
            verbose: bool = False,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            custom_domain: typing.Optional[str] = None,
        ):
            """Private method used internally.

            This method exposes some internal arguments (currently `mounts`) which are not in the public API.
            `mounts` is currently only used by modal shell (cli) to provide a function's mounts to the
            sandbox that runs the shell session.
            """
            ...

    _create: typing.ClassVar[___create_spec]

    class ___experimental_create_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *args: str,
            app: typing.Optional[modal.app.App] = None,
            name: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            image: typing.Optional[modal.image.Image] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            timeout: int = 300,
            idle_timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            cpu: typing.Union[float, tuple[float, float], None] = None,
            memory: typing.Union[int, tuple[int, int], None] = None,
            cloud: typing.Optional[str] = None,
            region: typing.Union[str, collections.abc.Sequence[str], None] = None,
            block_network: bool = False,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            i6pn: bool = False,
            volumes: dict[
                typing.Union[str, os.PathLike],
                typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
            ] = {},
            pty: bool = False,
            encrypted_ports: collections.abc.Sequence[int] = [],
            h2_ports: collections.abc.Sequence[int] = [],
            unencrypted_ports: collections.abc.Sequence[int] = [],
            proxy: typing.Optional[modal.proxy.Proxy] = None,
            readiness_probe: typing.Optional[Probe] = None,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
            include_oidc_identity_token: bool = False,
            verbose: bool = False,
            custom_domain: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
            _experimental_enable_snapshot: bool = False,
        ) -> Sandbox:
            """Create a sandbox using the V2 backend.

            Supported features include exec, encrypted tunnels, wait/poll/terminate,
            CPU and memory configuration, region placement, private IPv6 networking
            (i6pn), volumes, cloud bucket mounts (with static credentials via
            `secret=...` or `oidc_auth_role_arn`), OIDC identity tokens, proxies,
            filesystem snapshots, and custom domains (`custom_domain=...` allows
            connections to the sandbox via a subdomain of that parent domain rather
            than a default Modal domain; requires prior setup by Modal).

            `cpu` and `memory` accept either a scalar request or a `(request, limit)`
            tuple that additionally sets a hard limit (fractional CPU cores; memory
            in MiB), matching `Sandbox.create()`.

            Pass `_experimental_enable_snapshot=True` to create a sandbox that can be
            snapshotted with `._experimental_snapshot()`. Features like network
            file systems and GPUs are not supported.

            Set `i6pn=True` to enable private IPv6 networking so sandboxes in the same
            workspace can address each other directly at their `i6pn.modal.local`
            address. i6pn only connects sandboxes co-located on the same routable
            network, so pin every sandbox in the group to the same specific region
            (e.g. `region="us-east-1"`).

            V2 sandboxes created with this method are not currently returned by
            `Sandbox.list()`. A named sandbox can be looked up with
            `Sandbox._experimental_from_name(app_name, name)`; otherwise store
            `sandbox.object_id` and use `Sandbox.from_id(sandbox.object_id)` to
            reattach.
            """
            ...

        async def aio(
            self,
            /,
            *args: str,
            app: typing.Optional[modal.app.App] = None,
            name: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            image: typing.Optional[modal.image.Image] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            timeout: int = 300,
            idle_timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            cpu: typing.Union[float, tuple[float, float], None] = None,
            memory: typing.Union[int, tuple[int, int], None] = None,
            cloud: typing.Optional[str] = None,
            region: typing.Union[str, collections.abc.Sequence[str], None] = None,
            block_network: bool = False,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            inbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            i6pn: bool = False,
            volumes: dict[
                typing.Union[str, os.PathLike],
                typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
            ] = {},
            pty: bool = False,
            encrypted_ports: collections.abc.Sequence[int] = [],
            h2_ports: collections.abc.Sequence[int] = [],
            unencrypted_ports: collections.abc.Sequence[int] = [],
            proxy: typing.Optional[modal.proxy.Proxy] = None,
            readiness_probe: typing.Optional[Probe] = None,
            experimental_options: typing.Optional[dict[str, typing.Any]] = None,
            include_oidc_identity_token: bool = False,
            verbose: bool = False,
            custom_domain: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
            _experimental_enable_snapshot: bool = False,
        ) -> Sandbox:
            """Create a sandbox using the V2 backend.

            Supported features include exec, encrypted tunnels, wait/poll/terminate,
            CPU and memory configuration, region placement, private IPv6 networking
            (i6pn), volumes, cloud bucket mounts (with static credentials via
            `secret=...` or `oidc_auth_role_arn`), OIDC identity tokens, proxies,
            filesystem snapshots, and custom domains (`custom_domain=...` allows
            connections to the sandbox via a subdomain of that parent domain rather
            than a default Modal domain; requires prior setup by Modal).

            `cpu` and `memory` accept either a scalar request or a `(request, limit)`
            tuple that additionally sets a hard limit (fractional CPU cores; memory
            in MiB), matching `Sandbox.create()`.

            Pass `_experimental_enable_snapshot=True` to create a sandbox that can be
            snapshotted with `._experimental_snapshot()`. Features like network
            file systems and GPUs are not supported.

            Set `i6pn=True` to enable private IPv6 networking so sandboxes in the same
            workspace can address each other directly at their `i6pn.modal.local`
            address. i6pn only connects sandboxes co-located on the same routable
            network, so pin every sandbox in the group to the same specific region
            (e.g. `region="us-east-1"`).

            V2 sandboxes created with this method are not currently returned by
            `Sandbox.list()`. A named sandbox can be looked up with
            `Sandbox._experimental_from_name(app_name, name)`; otherwise store
            `sandbox.object_id` and use `Sandbox.from_id(sandbox.object_id)` to
            reattach.
            """
            ...

    _experimental_create: typing.ClassVar[___experimental_create_spec]

    def _get_metadata(self) -> modal_proto.api_pb2.SandboxHandleMetadata: ...
    def _hydrate_metadata(self, handle_metadata: typing.Optional[google.protobuf.message.Message]) -> None: ...
    def _hydrate_metadata_v2(self) -> None:
        """Wire up V2 stdio readers that read directly from the worker. Cheap
        to call eagerly: the router connection is opened lazily on first read.
        """
        ...

    def _initialize_from_other(self, other): ...
    def _initialize_from_empty(self): ...

    class __detach_spec(typing_extensions.Protocol):
        def __call__(self, /):
            """Disconnects your client from the sandbox and cleans up resources assoicated with the connection.

            Be sure to only call `detach` when you are done interacting with the sandbox. After calling `detach`,
            any operation using the Sandbox object is not guaranteed to work anymore. If you want to continue interacting
            with a running sandbox, use `Sandbox.from_id` to get a new Sandbox object.
            """
            ...

        async def aio(self, /):
            """Disconnects your client from the sandbox and cleans up resources assoicated with the connection.

            Be sure to only call `detach` when you are done interacting with the sandbox. After calling `detach`,
            any operation using the Sandbox object is not guaranteed to work anymore. If you want to continue interacting
            with a running sandbox, use `Sandbox.from_id` to get a new Sandbox object.
            """
            ...

    detach: __detach_spec

    @property
    def _client(self) -> modal.client.Client: ...
    @_client.setter
    def _client(self, value): ...
    def _ensure_attached(self): ...
    def _ensure_v1(self, method_name: str): ...

    class __from_name_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            app_name: str,
            name: str,
            *,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> Sandbox:
            """Get a running Sandbox by name from a deployed App.

            A Sandbox's name is the `name` argument passed to `Sandbox.create`.

            Args:
                app_name: Name of the deployed app to look up the sandbox under.
                name: Sandbox name to resolve.
                environment_name: Optional environment name for the lookup; defaults to the configured environment.
                client: Modal client to use for the RPC; defaults to `Client.from_env()` when omitted.

            Returns:
                A `Sandbox` handle for the running sandbox.

            Raises:
                NotFoundError: If no running sandbox exists with the given name.
            """
            ...

        async def aio(
            self,
            /,
            app_name: str,
            name: str,
            *,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> Sandbox:
            """Get a running Sandbox by name from a deployed App.

            A Sandbox's name is the `name` argument passed to `Sandbox.create`.

            Args:
                app_name: Name of the deployed app to look up the sandbox under.
                name: Sandbox name to resolve.
                environment_name: Optional environment name for the lookup; defaults to the configured environment.
                client: Modal client to use for the RPC; defaults to `Client.from_env()` when omitted.

            Returns:
                A `Sandbox` handle for the running sandbox.

            Raises:
                NotFoundError: If no running sandbox exists with the given name.
            """
            ...

    from_name: typing.ClassVar[__from_name_spec]

    class ___experimental_from_name_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            app_name: str,
            name: str,
            *,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> Sandbox:
            """Get a running V2 Sandbox by name from a deployed App.

            This looks up V2 sandboxes, ie sandboxes created via `modal.Sandbox._experimental_create`.

            Args:
                app_name: Name of the deployed app to look up the sandbox under.
                name: Sandbox name to resolve.
                environment_name: Optional environment name for the lookup; defaults to the configured environment.
                client: Modal client to use for the RPC; defaults to `Client.from_env()` when omitted.

            Returns:
                A `Sandbox` handle for the running sandbox.

            Raises:
                NotFoundError: If no running sandbox exists with the given name.
            """
            ...

        async def aio(
            self,
            /,
            app_name: str,
            name: str,
            *,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> Sandbox:
            """Get a running V2 Sandbox by name from a deployed App.

            This looks up V2 sandboxes, ie sandboxes created via `modal.Sandbox._experimental_create`.

            Args:
                app_name: Name of the deployed app to look up the sandbox under.
                name: Sandbox name to resolve.
                environment_name: Optional environment name for the lookup; defaults to the configured environment.
                client: Modal client to use for the RPC; defaults to `Client.from_env()` when omitted.

            Returns:
                A `Sandbox` handle for the running sandbox.

            Raises:
                NotFoundError: If no running sandbox exists with the given name.
            """
            ...

    _experimental_from_name: typing.ClassVar[___experimental_from_name_spec]

    class __from_id_spec(typing_extensions.Protocol):
        def __call__(self, /, sandbox_id: str, client: typing.Optional[modal.client.Client] = None) -> Sandbox:
            """Construct a Sandbox from an id and look up the Sandbox result.

            The ID of a Sandbox object can be accessed using `.object_id`.

            Args:
                sandbox_id: Sandbox object ID to attach to.
                client: Modal client to use for the lookup; defaults to the environment client when omitted.

            Returns:
                A `Sandbox` handle with any available result metadata populated from the server.
            """
            ...

        async def aio(self, /, sandbox_id: str, client: typing.Optional[modal.client.Client] = None) -> Sandbox:
            """Construct a Sandbox from an id and look up the Sandbox result.

            The ID of a Sandbox object can be accessed using `.object_id`.

            Args:
                sandbox_id: Sandbox object ID to attach to.
                client: Modal client to use for the lookup; defaults to the environment client when omitted.

            Returns:
                A `Sandbox` handle with any available result metadata populated from the server.
            """
            ...

    from_id: typing.ClassVar[__from_id_spec]

    class __get_tags_spec(typing_extensions.Protocol):
        def __call__(self, /) -> dict[str, str]:
            """Fetches any tags (key-value pairs) currently attached to this Sandbox from the server.

            Returns:
                Tags as a map from tag name to tag value.
            """
            ...

        async def aio(self, /) -> dict[str, str]:
            """Fetches any tags (key-value pairs) currently attached to this Sandbox from the server.

            Returns:
                Tags as a map from tag name to tag value.
            """
            ...

    get_tags: __get_tags_spec

    class __set_tags_spec(typing_extensions.Protocol):
        def __call__(self, /, tags: dict[str, str], *, client: typing.Optional[modal.client.Client] = None) -> None:
            """Set tags (key-value pairs) on the Sandbox. Tags can be used to filter results in `Sandbox.list`.

            Setting tags replaces the Sandbox's entire tag set; passing an empty dict clears all tags.

            Args:
                tags: Tag names and values to set on this sandbox.
                client: Deprecated. Prefer setting the client when creating or re-attaching to the sandbox.
            """
            ...

        async def aio(self, /, tags: dict[str, str], *, client: typing.Optional[modal.client.Client] = None) -> None:
            """Set tags (key-value pairs) on the Sandbox. Tags can be used to filter results in `Sandbox.list`.

            Setting tags replaces the Sandbox's entire tag set; passing an empty dict clears all tags.

            Args:
                tags: Tag names and values to set on this sandbox.
                client: Deprecated. Prefer setting the client when creating or re-attaching to the sandbox.
            """
            ...

    set_tags: __set_tags_spec

    class ___experimental_set_name_spec(typing_extensions.Protocol):
        def __call__(self, /, name: str) -> None:
            """Assign a name to a running V2 Sandbox that was created without one.

            This is only supported for V2 sandboxes, ie sandboxes created via
            `modal.Sandbox._experimental_create`. A name may only be set once and
            only on a Sandbox that has never had one; afterwards the Sandbox can
            be looked up with `Sandbox._experimental_from_name(app_name, name)`.

            Args:
                name: Name to assign to the Sandbox. Must be unique within the App.

            Raises:
                AlreadyExistsError: If another running Sandbox in the App already holds the name.
                ConflictError: If the Sandbox already has a name or is no longer running.
            """
            ...

        async def aio(self, /, name: str) -> None:
            """Assign a name to a running V2 Sandbox that was created without one.

            This is only supported for V2 sandboxes, ie sandboxes created via
            `modal.Sandbox._experimental_create`. A name may only be set once and
            only on a Sandbox that has never had one; afterwards the Sandbox can
            be looked up with `Sandbox._experimental_from_name(app_name, name)`.

            Args:
                name: Name to assign to the Sandbox. Must be unique within the App.

            Raises:
                AlreadyExistsError: If another running Sandbox in the App already holds the name.
                ConflictError: If the Sandbox already has a name or is no longer running.
            """
            ...

    _experimental_set_name: ___experimental_set_name_spec

    class ___experimental_set_outbound_network_policy_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        ) -> None:
            """Replace the outbound network policy of a running Sandbox.

            Established connections that the new policy no longer permits are
            terminated.

            Args:
                outbound_cidr_allowlist: List of CIDRs the Sandbox is allowed to access. If None, all CIDRs are allowed.
                outbound_domain_allowlist: List of domain names the Sandbox is allowed to access. Supports
                    wildcard prefixes (``*.``); a bare ``"*"`` allows all domains.
            """
            ...

        async def aio(
            self,
            /,
            *,
            outbound_cidr_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
            outbound_domain_allowlist: typing.Optional[collections.abc.Sequence[str]] = None,
        ) -> None:
            """Replace the outbound network policy of a running Sandbox.

            Established connections that the new policy no longer permits are
            terminated.

            Args:
                outbound_cidr_allowlist: List of CIDRs the Sandbox is allowed to access. If None, all CIDRs are allowed.
                outbound_domain_allowlist: List of domain names the Sandbox is allowed to access. Supports
                    wildcard prefixes (``*.``); a bare ``"*"`` allows all domains.
            """
            ...

    _experimental_set_outbound_network_policy: ___experimental_set_outbound_network_policy_spec

    class ___experimental_get_exit_snapshot_spec(typing_extensions.Protocol):
        def __call__(self, /, timeout: typing.Optional[float] = 60) -> modal.image.Image:
            """Get the exit filesystem snapshot image.

            Args:
                timeout: Client-side deadline in seconds (default 60). Use `None` to
                    poll until the snapshot reaches a terminal state. Use `0` to
                    perform an immediate check.

            Returns:
                The exit snapshot Image.

            Raises:
                InvalidError: If `timeout` is negative, or if exit snapshot is not
                    enabled for the sandbox.
                TimeoutError: If `timeout` elapses before the snapshot reaches a
                    terminal state. This includes `timeout=0` when the snapshot is
                    still pending.
                SnapshotCreationError: If no exit snapshot image will be produced.
                NotFoundError: If the sandbox does not exist.
                PermissionDeniedError: If the caller cannot access the sandbox.
                InternalError: If persisted snapshot state is malformed.
                ServiceError: If a transient client/server communication failure occurs.
            """
            ...

        async def aio(self, /, timeout: typing.Optional[float] = 60) -> modal.image.Image:
            """Get the exit filesystem snapshot image.

            Args:
                timeout: Client-side deadline in seconds (default 60). Use `None` to
                    poll until the snapshot reaches a terminal state. Use `0` to
                    perform an immediate check.

            Returns:
                The exit snapshot Image.

            Raises:
                InvalidError: If `timeout` is negative, or if exit snapshot is not
                    enabled for the sandbox.
                TimeoutError: If `timeout` elapses before the snapshot reaches a
                    terminal state. This includes `timeout=0` when the snapshot is
                    still pending.
                SnapshotCreationError: If no exit snapshot image will be produced.
                NotFoundError: If the sandbox does not exist.
                PermissionDeniedError: If the caller cannot access the sandbox.
                InternalError: If persisted snapshot state is malformed.
                ServiceError: If a transient client/server communication failure occurs.
            """
            ...

    _experimental_get_exit_snapshot: ___experimental_get_exit_snapshot_spec

    class __snapshot_filesystem_spec(typing_extensions.Protocol):
        def __call__(self, /, timeout: int = 55, *, ttl: typing.Optional[int] = 2592000) -> modal.image.Image:
            """Snapshot the filesystem of the Sandbox.

            Args:
                timeout:
                    Maximum time in seconds to wait for the snapshot operation. If the snapshot does not return within
                    that window, the call is cancelled and `modal.exception.TimeoutError` is raised.
                ttl:
                    The resulting Image is retained for `ttl` seconds (default: 30 days). Pass `ttl=None` to retain
                    the image indefinitely.

            Returns:
                An [`Image`](https://modal.com/docs/sdk/py/latest/Image) object which can be used to spawn a new
                Sandbox with the same filesystem.
            """
            ...

        async def aio(self, /, timeout: int = 55, *, ttl: typing.Optional[int] = 2592000) -> modal.image.Image:
            """Snapshot the filesystem of the Sandbox.

            Args:
                timeout:
                    Maximum time in seconds to wait for the snapshot operation. If the snapshot does not return within
                    that window, the call is cancelled and `modal.exception.TimeoutError` is raised.
                ttl:
                    The resulting Image is retained for `ttl` seconds (default: 30 days). Pass `ttl=None` to retain
                    the image indefinitely.

            Returns:
                An [`Image`](https://modal.com/docs/sdk/py/latest/Image) object which can be used to spawn a new
                Sandbox with the same filesystem.
            """
            ...

    snapshot_filesystem: __snapshot_filesystem_spec

    class ___legacy_snapshot_filesystem_spec(typing_extensions.Protocol):
        def __call__(self, /, timeout: int = 55) -> modal.image.Image: ...
        async def aio(self, /, timeout: int = 55) -> modal.image.Image: ...

    _legacy_snapshot_filesystem: ___legacy_snapshot_filesystem_spec

    class __mount_image_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            path: typing.Union[pathlib.PurePosixPath, str],
            image: modal.image.Image,
            *,
            _experimental_encryption_key: typing.Optional[bytes] = None,
        ):
            """Mount an Image at a specified path in a running Sandbox.

            `path` should be a directory that is **not** the root path (`/`). If the path doesn't exist
            it will be created. If it exists and contains data, the previous directory will be replaced
            by the mount.

            The `image` argument supports any Image that has an object ID, including:
            - Images built using `image.build()`
            - Images referenced by ID, e.g. `Image.from_id(...)`
            - Filesystem/directory snapshots, e.g. created by `.snapshot_directory()` or `.snapshot_filesystem()`
            - Empty images created with `Image.from_scratch()`

            Args:
                path: Absolute mount point directory inside the sandbox (not `/`).
                image: Image to mount at `path` (must be built, referenced by ID, or snapshot-based as described above).


            Examples:
                ```py notest
                user_project_snapshot: Image = sandbox_session_1.snapshot_directory("/user_project")

                # You can later mount this snapshot to another Sandbox:
                sandbox_session_2 = modal.Sandbox.create(...)
                sandbox_session_2.mount_image("/user_project", user_project_snapshot)
                sandbox_session_2.filesystem.list_files("/user_project")
                ```
            """
            ...

        async def aio(
            self,
            /,
            path: typing.Union[pathlib.PurePosixPath, str],
            image: modal.image.Image,
            *,
            _experimental_encryption_key: typing.Optional[bytes] = None,
        ):
            """Mount an Image at a specified path in a running Sandbox.

            `path` should be a directory that is **not** the root path (`/`). If the path doesn't exist
            it will be created. If it exists and contains data, the previous directory will be replaced
            by the mount.

            The `image` argument supports any Image that has an object ID, including:
            - Images built using `image.build()`
            - Images referenced by ID, e.g. `Image.from_id(...)`
            - Filesystem/directory snapshots, e.g. created by `.snapshot_directory()` or `.snapshot_filesystem()`
            - Empty images created with `Image.from_scratch()`

            Args:
                path: Absolute mount point directory inside the sandbox (not `/`).
                image: Image to mount at `path` (must be built, referenced by ID, or snapshot-based as described above).


            Examples:
                ```py notest
                user_project_snapshot: Image = sandbox_session_1.snapshot_directory("/user_project")

                # You can later mount this snapshot to another Sandbox:
                sandbox_session_2 = modal.Sandbox.create(...)
                sandbox_session_2.mount_image("/user_project", user_project_snapshot)
                sandbox_session_2.filesystem.list_files("/user_project")
                ```
            """
            ...

    mount_image: __mount_image_spec

    class __unmount_image_spec(typing_extensions.Protocol):
        def __call__(self, /, path: typing.Union[pathlib.PurePosixPath, str]):
            """Unmount a previously mounted Image from a running Sandbox.

            `path` must be the exact mount point that was passed to `.mount_image()`.
            After unmounting, the underlying Sandbox filesystem at that path becomes
            visible again.

            Args:
                path: Absolute mount point directory to unmount.
            """
            ...

        async def aio(self, /, path: typing.Union[pathlib.PurePosixPath, str]):
            """Unmount a previously mounted Image from a running Sandbox.

            `path` must be the exact mount point that was passed to `.mount_image()`.
            After unmounting, the underlying Sandbox filesystem at that path becomes
            visible again.

            Args:
                path: Absolute mount point directory to unmount.
            """
            ...

    unmount_image: __unmount_image_spec

    class __snapshot_directory_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            path: typing.Union[pathlib.PurePosixPath, str],
            *,
            timeout: int = 55,
            ttl: typing.Optional[int] = 2592000,
            _experimental_encryption_key: typing.Optional[bytes] = None,
        ) -> modal.image.Image:
            """Snapshot a directory in a running Sandbox, creating a new Image with its content.

            `timeout` If the snapshot does not return within that window, the call is cancelled
            and `modal.exception.TimeoutError` is raised.

            `ttl` The resulting Image is retained for `ttl` seconds (default: 30 days)
            Pass `ttl=None` to retain the image indefinitely.

            Args:
                path: Absolute path of the directory inside the sandbox to snapshot.

            Returns:
                An `Image` containing the directory contents.

            Examples:
                ```py notest
                user_project_snapshot: Image = sandbox_session_1.snapshot_directory("/user_project")

                # You can later mount this snapshot to another Sandbox:
                sandbox_session_2 = modal.Sandbox.create(...)
                sandbox_session_2.mount_image("/user_project", user_project_snapshot)
                sandbox_session_2.filesystem.list_files("/user_project")
                ```
            """
            ...

        async def aio(
            self,
            /,
            path: typing.Union[pathlib.PurePosixPath, str],
            *,
            timeout: int = 55,
            ttl: typing.Optional[int] = 2592000,
            _experimental_encryption_key: typing.Optional[bytes] = None,
        ) -> modal.image.Image:
            """Snapshot a directory in a running Sandbox, creating a new Image with its content.

            `timeout` If the snapshot does not return within that window, the call is cancelled
            and `modal.exception.TimeoutError` is raised.

            `ttl` The resulting Image is retained for `ttl` seconds (default: 30 days)
            Pass `ttl=None` to retain the image indefinitely.

            Args:
                path: Absolute path of the directory inside the sandbox to snapshot.

            Returns:
                An `Image` containing the directory contents.

            Examples:
                ```py notest
                user_project_snapshot: Image = sandbox_session_1.snapshot_directory("/user_project")

                # You can later mount this snapshot to another Sandbox:
                sandbox_session_2 = modal.Sandbox.create(...)
                sandbox_session_2.mount_image("/user_project", user_project_snapshot)
                sandbox_session_2.filesystem.list_files("/user_project")
                ```
            """
            ...

    snapshot_directory: __snapshot_directory_spec

    class __wait_spec(typing_extensions.Protocol):
        def __call__(self, /, raise_on_termination: bool = True):
            """Wait for the Sandbox to finish running.

            Args:
                raise_on_termination: If True, raise when the sandbox is terminated externally.
            """
            ...

        async def aio(self, /, raise_on_termination: bool = True):
            """Wait for the Sandbox to finish running.

            Args:
                raise_on_termination: If True, raise when the sandbox is terminated externally.
            """
            ...

    wait: __wait_spec

    class __wait_until_ready_spec(typing_extensions.Protocol):
        def __call__(self, /, *, timeout: int = 300) -> None:
            """Wait for the Sandbox readiness probe to report that the Sandbox is ready.

            The Sandbox must be configured with a `readiness_probe` in order to use this method.

            Args:
                timeout: Maximum time in seconds to wait for readiness.


            Examples:
                ```py notest
                app = modal.App.lookup('sandbox-wait-until-ready', create_if_missing=True)
                sandbox = modal.Sandbox.create(
                    "python3", "-m", "http.server", "8080",
                    readiness_probe=modal.Probe.with_tcp(8080),
                    app=app,
                )
                sandbox.wait_until_ready()
                ```
            """
            ...

        async def aio(self, /, *, timeout: int = 300) -> None:
            """Wait for the Sandbox readiness probe to report that the Sandbox is ready.

            The Sandbox must be configured with a `readiness_probe` in order to use this method.

            Args:
                timeout: Maximum time in seconds to wait for readiness.


            Examples:
                ```py notest
                app = modal.App.lookup('sandbox-wait-until-ready', create_if_missing=True)
                sandbox = modal.Sandbox.create(
                    "python3", "-m", "http.server", "8080",
                    readiness_probe=modal.Probe.with_tcp(8080),
                    app=app,
                )
                sandbox.wait_until_ready()
                ```
            """
            ...

    wait_until_ready: __wait_until_ready_spec

    class __tunnels_spec(typing_extensions.Protocol):
        def __call__(self, /, timeout: int = 50) -> dict[int, modal._tunnel.Tunnel]:
            """Get Tunnel metadata for the sandbox.

            NOTE: Previous to client [v0.64.153](https://modal.com/docs/sdk/py/changelog#064153-2024-09-30), this
            returned a list of `TunnelData` objects.

            Args:
                timeout: Maximum time in seconds to wait for tunnel metadata when not already cached.

            Returns:
                A dictionary mapping container port to `Tunnel` metadata.

            Raises:
                SandboxTimeoutError: If the tunnels are not available after the timeout.
            """
            ...

        async def aio(self, /, timeout: int = 50) -> dict[int, modal._tunnel.Tunnel]:
            """Get Tunnel metadata for the sandbox.

            NOTE: Previous to client [v0.64.153](https://modal.com/docs/sdk/py/changelog#064153-2024-09-30), this
            returned a list of `TunnelData` objects.

            Args:
                timeout: Maximum time in seconds to wait for tunnel metadata when not already cached.

            Returns:
                A dictionary mapping container port to `Tunnel` metadata.

            Raises:
                SandboxTimeoutError: If the tunnels are not available after the timeout.
            """
            ...

    tunnels: __tunnels_spec

    class __create_connect_token_spec(typing_extensions.Protocol):
        def __call__(
            self, /, user_metadata: typing.Union[str, dict[str, typing.Any], None] = None, port: int = 8080
        ) -> modal.types.SandboxConnectCredentials:
            """Create a token for making HTTP connections to the Sandbox.

            Accepts an optional user_metadata string or dict to associate with the token. This metadata
            will be added to the headers by the proxy when forwarding requests to the Sandbox.
            Also accepts a port that requests will be routed to.

            Args:
                user_metadata: Optional JSON-serializable metadata or string stored with the connect token.
                port: Optional container port that requests are routed to when using this token.

            Returns:
                URL and token credentials for connecting to the sandbox over HTTP.
            """
            ...

        async def aio(
            self, /, user_metadata: typing.Union[str, dict[str, typing.Any], None] = None, port: int = 8080
        ) -> modal.types.SandboxConnectCredentials:
            """Create a token for making HTTP connections to the Sandbox.

            Accepts an optional user_metadata string or dict to associate with the token. This metadata
            will be added to the headers by the proxy when forwarding requests to the Sandbox.
            Also accepts a port that requests will be routed to.

            Args:
                user_metadata: Optional JSON-serializable metadata or string stored with the connect token.
                port: Optional container port that requests are routed to when using this token.

            Returns:
                URL and token credentials for connecting to the sandbox over HTTP.
            """
            ...

    create_connect_token: __create_connect_token_spec

    class __reload_volumes_spec(typing_extensions.Protocol):
        def __call__(self, /, *, timeout: int = 55) -> None:
            """Reload all Volumes mounted in the Sandbox.

            Added in v1.1.0.

            Blocks until the Volumes have been reloaded, bounded by `timeout` (55 seconds by default). If the reload
            does not complete within that window, `modal.exception.TimeoutError` is raised; note that the reload may
            still complete in the background.

            Args:
                timeout: Maximum time in seconds to wait for the reload. Must be positive.
            """
            ...

        async def aio(self, /, *, timeout: int = 55) -> None:
            """Reload all Volumes mounted in the Sandbox.

            Added in v1.1.0.

            Blocks until the Volumes have been reloaded, bounded by `timeout` (55 seconds by default). If the reload
            does not complete within that window, `modal.exception.TimeoutError` is raised; note that the reload may
            still complete in the background.

            Args:
                timeout: Maximum time in seconds to wait for the reload. Must be positive.
            """
            ...

    reload_volumes: __reload_volumes_spec

    class __terminate_spec(typing_extensions.Protocol):
        @typing.overload
        def __call__(self, /, *, wait: typing.Literal[True]) -> int: ...
        @typing.overload
        def __call__(self, /, *, wait: typing.Literal[False] = False) -> None: ...
        @typing.overload
        async def aio(self, /, *, wait: typing.Literal[True]) -> int: ...
        @typing.overload
        async def aio(self, /, *, wait: typing.Literal[False] = False) -> None: ...

    terminate: __terminate_spec

    class __poll_spec(typing_extensions.Protocol):
        def __call__(self, /) -> typing.Optional[int]:
            """Check if the Sandbox has finished running.

            Returns:
                `None` if the Sandbox is still running, otherwise the exit code.
            """
            ...

        async def aio(self, /) -> typing.Optional[int]:
            """Check if the Sandbox has finished running.

            Returns:
                `None` if the Sandbox is still running, otherwise the exit code.
            """
            ...

    poll: __poll_spec

    class ___get_task_id_spec(typing_extensions.Protocol):
        def __call__(self, /, raise_if_task_complete=False) -> str: ...
        async def aio(self, /, raise_if_task_complete=False) -> str: ...

    _get_task_id: ___get_task_id_spec

    class ___get_command_router_client_spec(typing_extensions.Protocol):
        def __call__(self, /, task_id: str) -> modal._utils.task_command_router_client.TaskCommandRouterClient: ...
        async def aio(self, /, task_id: str) -> modal._utils.task_command_router_client.TaskCommandRouterClient: ...

    _get_command_router_client: ___get_command_router_client_spec

    @property
    def _experimental_sidecars(self) -> SidecarManager:
        """Manage sidecar containers running in this Sandbox."""
        ...

    class __exec_spec(typing_extensions.Protocol):
        @typing.overload
        def __call__(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[True] = True,
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            _pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        ) -> modal.container_process.ContainerProcess[str]: ...
        @typing.overload
        def __call__(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[False] = False,
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            _pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        ) -> modal.container_process.ContainerProcess[bytes]: ...
        @typing.overload
        async def aio(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[True] = True,
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            _pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        ) -> modal.container_process.ContainerProcess[str]: ...
        @typing.overload
        async def aio(
            self,
            /,
            *args: str,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: typing.Literal[False] = False,
            bufsize: typing.Literal[-1, 1] = -1,
            pty: bool = False,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            _pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
        ) -> modal.container_process.ContainerProcess[bytes]: ...

    exec: __exec_spec

    class ___exec_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *args: str,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: bool = True,
            bufsize: typing.Literal[-1, 1] = -1,
            container_id: typing.Optional[str] = None,
        ) -> typing.Union[
            modal.container_process.ContainerProcess[bytes], modal.container_process.ContainerProcess[str]
        ]:
            """Private method used internally.

            This method exposes some internal arguments (currently `pty_info`) which are not in the public API.
            """
            ...

        async def aio(
            self,
            /,
            *args: str,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            env: typing.Optional[dict[str, typing.Optional[str]]] = None,
            secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
            text: bool = True,
            bufsize: typing.Literal[-1, 1] = -1,
            container_id: typing.Optional[str] = None,
        ) -> typing.Union[
            modal.container_process.ContainerProcess[bytes], modal.container_process.ContainerProcess[str]
        ]:
            """Private method used internally.

            This method exposes some internal arguments (currently `pty_info`) which are not in the public API.
            """
            ...

    _exec: ___exec_spec

    class ___exec_through_command_router_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *args: str,
            task_id: str,
            command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            secret_ids: typing.Optional[collections.abc.Collection[str]] = None,
            env: typing.Optional[dict[str, str]] = None,
            text: bool = True,
            bufsize: typing.Literal[-1, 1] = -1,
            runtime_debug: bool = False,
            container_id: typing.Optional[str] = None,
        ) -> typing.Union[
            modal.container_process.ContainerProcess[bytes], modal.container_process.ContainerProcess[str]
        ]:
            """Execute a command through a task command router running on the Modal worker."""
            ...

        async def aio(
            self,
            /,
            *args: str,
            task_id: str,
            command_router_client: modal._utils.task_command_router_client.TaskCommandRouterClient,
            pty_info: typing.Optional[modal_proto.api_pb2.PTYInfo] = None,
            stdout: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            stderr: modal.stream_type.StreamType = modal.stream_type.StreamType.PIPE,
            timeout: typing.Optional[int] = None,
            workdir: typing.Optional[str] = None,
            secret_ids: typing.Optional[collections.abc.Collection[str]] = None,
            env: typing.Optional[dict[str, str]] = None,
            text: bool = True,
            bufsize: typing.Literal[-1, 1] = -1,
            runtime_debug: bool = False,
            container_id: typing.Optional[str] = None,
        ) -> typing.Union[
            modal.container_process.ContainerProcess[bytes], modal.container_process.ContainerProcess[str]
        ]:
            """Execute a command through a task command router running on the Modal worker."""
            ...

    _exec_through_command_router: ___exec_through_command_router_spec

    class ___experimental_snapshot_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal.snapshot.SandboxSnapshot: ...
        async def aio(self, /) -> modal.snapshot.SandboxSnapshot: ...

    _experimental_snapshot: ___experimental_snapshot_spec

    class ___experimental_from_snapshot_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            snapshot: modal.snapshot.SandboxSnapshot,
            client: typing.Optional[modal.client.Client] = None,
            *,
            name: typing.Optional[str] = _DEFAULT_SANDBOX_NAME_OVERRIDE,
        ):
            """Restore a Sandbox from a memory snapshot.

            The restore targets the same backend the snapshot was taken from. A V1
            snapshot restores as a V1 sandbox, a V2 snapshot as a V2 sandbox.
            """
            ...

        async def aio(
            self,
            /,
            snapshot: modal.snapshot.SandboxSnapshot,
            client: typing.Optional[modal.client.Client] = None,
            *,
            name: typing.Optional[str] = _DEFAULT_SANDBOX_NAME_OVERRIDE,
        ):
            """Restore a Sandbox from a memory snapshot.

            The restore targets the same backend the snapshot was taken from. A V1
            snapshot restores as a V1 sandbox, a V2 snapshot as a V2 sandbox.
            """
            ...

    _experimental_from_snapshot: typing.ClassVar[___experimental_from_snapshot_spec]

    class ___experimental_from_snapshot_v2_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            snapshot: modal.snapshot.SandboxSnapshot,
            client: modal.client.Client,
            *,
            override_type: int,
            name_override: typing.Optional[str],
        ) -> Sandbox: ...
        async def aio(
            self,
            /,
            snapshot: modal.snapshot.SandboxSnapshot,
            client: modal.client.Client,
            *,
            override_type: int,
            name_override: typing.Optional[str],
        ) -> Sandbox: ...

    _experimental_from_snapshot_v2: typing.ClassVar[___experimental_from_snapshot_v2_spec]

    @property
    def filesystem(self) -> modal.sandbox_fs.SandboxFilesystem:
        """Namespace for Sandbox filesystem APIs."""
        ...

    class __open_spec(typing_extensions.Protocol):
        @typing.overload
        def __call__(self, /, path: str) -> modal.file_io.FileIO[str]: ...
        @typing.overload
        def __call__(self, /, path: str, mode: _typeshed.OpenTextMode) -> modal.file_io.FileIO[str]: ...
        @typing.overload
        def __call__(self, /, path: str, mode: _typeshed.OpenBinaryMode) -> modal.file_io.FileIO[bytes]: ...
        @typing.overload
        async def aio(self, /, path: str) -> modal.file_io.FileIO[str]: ...
        @typing.overload
        async def aio(self, /, path: str, mode: _typeshed.OpenTextMode) -> modal.file_io.FileIO[str]: ...
        @typing.overload
        async def aio(self, /, path: str, mode: _typeshed.OpenBinaryMode) -> modal.file_io.FileIO[bytes]: ...

    open: __open_spec

    class __ls_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str) -> list[str]:
            """[Alpha] List the contents of a directory in the Sandbox.

            **Deprecated (2026-04-15):** Use `Sandbox.filesystem.list_files()` instead for improved reliability.

            Args:
                path: Absolute directory path inside the sandbox.

            Returns:
                Entry names in the directory as a list of strings.
            """
            ...

        async def aio(self, /, path: str) -> list[str]:
            """[Alpha] List the contents of a directory in the Sandbox.

            **Deprecated (2026-04-15):** Use `Sandbox.filesystem.list_files()` instead for improved reliability.

            Args:
                path: Absolute directory path inside the sandbox.

            Returns:
                Entry names in the directory as a list of strings.
            """
            ...

    ls: __ls_spec

    class __mkdir_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str, parents: bool = False) -> None:
            """[Alpha] Create a new directory in the Sandbox.

            **Deprecated (2026-04-15):** Use `Sandbox.filesystem.make_directory()` instead for improved reliability.
            """
            ...

        async def aio(self, /, path: str, parents: bool = False) -> None:
            """[Alpha] Create a new directory in the Sandbox.

            **Deprecated (2026-04-15):** Use `Sandbox.filesystem.make_directory()` instead for improved reliability.
            """
            ...

    mkdir: __mkdir_spec

    class __rm_spec(typing_extensions.Protocol):
        def __call__(self, /, path: str, recursive: bool = False) -> None:
            """[Alpha] Remove a file or directory in the Sandbox.

            **Deprecated (2026-04-15):** Use `Sandbox.filesystem.remove()` instead for improved reliability.
            """
            ...

        async def aio(self, /, path: str, recursive: bool = False) -> None:
            """[Alpha] Remove a file or directory in the Sandbox.

            **Deprecated (2026-04-15):** Use `Sandbox.filesystem.remove()` instead for improved reliability.
            """
            ...

    rm: __rm_spec

    class __watch_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            path: str,
            filter: typing.Optional[list[modal.types.FileWatchEventType]] = None,
            recursive: typing.Optional[bool] = None,
            timeout: typing.Optional[int] = None,
        ) -> typing.Iterator[modal.types.FileWatchEvent]:
            """[Alpha] Watch a file or directory in the Sandbox for changes.

            **Deprecated (2026-05-08):** Use `Sandbox.filesystem.watch()` instead for improved reliability.

            Args:
                path: Absolute path to watch.
                filter: Optional list of event types to include.
                recursive: Whether to watch subdirectories; None uses server defaults.
                timeout: Optional timeout for the watch stream.

            Returns:
                An async iterator of `FileWatchEvent` values.
            """
            ...

        def aio(
            self,
            /,
            path: str,
            filter: typing.Optional[list[modal.types.FileWatchEventType]] = None,
            recursive: typing.Optional[bool] = None,
            timeout: typing.Optional[int] = None,
        ) -> collections.abc.AsyncIterator[modal.types.FileWatchEvent]:
            """[Alpha] Watch a file or directory in the Sandbox for changes.

            **Deprecated (2026-05-08):** Use `Sandbox.filesystem.watch()` instead for improved reliability.

            Args:
                path: Absolute path to watch.
                filter: Optional list of event types to include.
                recursive: Whether to watch subdirectories; None uses server defaults.
                timeout: Optional timeout for the watch stream.

            Returns:
                An async iterator of `FileWatchEvent` values.
            """
            ...

    watch: __watch_spec

    @property
    def stdout(self) -> modal.io_streams.StreamReader[str]:
        """[`StreamReader`](https://modal.com/docs/sdk/py/latest/io_streams#streamreader)
        for the sandbox's stdout stream.

        Returns:
            Stream reader for sandbox stdout.
        """
        ...

    @property
    def stderr(self) -> modal.io_streams.StreamReader[str]:
        """[`StreamReader`](https://modal.com/docs/sdk/py/latest/io_streams#streamreader)
        for the Sandbox's stderr stream.

        Returns:
            Stream reader for sandbox stderr.
        """
        ...

    @property
    def stdin(self) -> modal.io_streams.StreamWriter:
        """[`StreamWriter`](https://modal.com/docs/sdk/py/latest/io_streams#streamwriter)
        for the Sandbox's stdin stream.

        Returns:
            Stream writer for sandbox stdin.
        """
        ...

    @property
    def returncode(self) -> typing.Optional[int]:
        """Return code of the Sandbox process if it has finished running, else `None`.

        Returns:
            Exit code when the sandbox process has completed, otherwise None.
        """
        ...

    class __list_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            app_id: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> typing.Generator[Sandbox, None, None]:
            """List all Sandboxes for the current Environment or App ID (if specified). If tags are specified, only
            Sandboxes that have at least those tags are returned.

            Args:
                app_id: If set, restrict results to sandboxes under this app ID.
                tags: If set, only sandboxes containing at least these tags are returned.
                client: Modal client to use for listing; defaults to `Client.from_env()` when omitted.

            Returns:
                An async generator yielding `Sandbox` objects.
            """
            ...

        def aio(
            self,
            /,
            *,
            app_id: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> collections.abc.AsyncGenerator[Sandbox, None]:
            """List all Sandboxes for the current Environment or App ID (if specified). If tags are specified, only
            Sandboxes that have at least those tags are returned.

            Args:
                app_id: If set, restrict results to sandboxes under this app ID.
                tags: If set, only sandboxes containing at least these tags are returned.
                client: Modal client to use for listing; defaults to `Client.from_env()` when omitted.

            Returns:
                An async generator yielding `Sandbox` objects.
            """
            ...

    list: typing.ClassVar[__list_spec]

    class ___experimental_list_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            app_id: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> typing.Generator[Sandbox, None, None]:
            """List v2 Sandboxes in an App.

            This function lists v2 sandboxes, ie sandboxes created via modal.Sandbox._experimental_create.

            Args:
                app_id: The App to list Sandboxes under.
                tags: If set, only sandboxes containing at least these tags are returned.
                client: Optional client to use for the session.

            Yields:
                `Sandbox` objects that are currently running in the App.
            """
            ...

        def aio(
            self,
            /,
            *,
            app_id: typing.Optional[str] = None,
            tags: typing.Optional[dict[str, str]] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> collections.abc.AsyncGenerator[Sandbox, None]:
            """List v2 Sandboxes in an App.

            This function lists v2 sandboxes, ie sandboxes created via modal.Sandbox._experimental_create.

            Args:
                app_id: The App to list Sandboxes under.
                tags: If set, only sandboxes containing at least these tags are returned.
                client: Optional client to use for the session.

            Yields:
                `Sandbox` objects that are currently running in the App.
            """
            ...

    _experimental_list: typing.ClassVar[___experimental_list_spec]

_default_image: modal._image._Image

_MAIN_CONTAINER_NAME: str
