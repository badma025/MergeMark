import collections.abc
import datetime
import google.protobuf.message
import modal._load_context
import modal._object
import modal.client
import modal.object
import modal.types
import modal_proto.api_pb2
import synchronicity
import typing
import typing_extensions

class _SecretManager:
    """Namespace with methods for managing named Secret objects."""
    async def create(
        self,
        name: str,
        env_dict: dict[str, str],
        *,
        allow_existing: bool = False,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
    ) -> None:
        """Create a new named Secret in the workspace environment.

        This does not return a local handle; use `modal.Secret.from_name` to look up the Secret after creation.

        Added in v1.1.2.

        Args:
            name: Name for the new Secret.
            env_dict: Environment variable keys and values stored in the Secret.
            allow_existing: If True, do nothing when a Secret with this name already exists (existing values are kept).
            environment_name: Environment to create in; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.

        Examples:
            ```python notest
            contents = {"MY_KEY": "my-value", "MY_OTHER_KEY": "my-other-value"}
            modal.Secret.objects.create("my-secret", contents)
            ```

            Secrets will be created in the active environment, or another one can be specified:

            ```python notest
            modal.Secret.objects.create("my-secret", contents, environment_name="dev")
            ```

            By default, an error will be raised if the Secret already exists, but passing
            `allow_existing=True` will make the creation attempt a no-op in this case.
            If the `env_dict` data differs from the existing Secret, it will be ignored.

            ```python notest
            modal.Secret.objects.create("my-secret", contents, allow_existing=True)
            ```

            Note that this method does not return a local instance of the Secret. You can use
            `modal.Secret.from_name` to perform a lookup after creation.
        """
        ...

    async def list(
        self,
        *,
        max_objects: typing.Optional[int] = None,
        created_before: typing.Union[datetime.datetime, str, None] = None,
        environment_name: str = "",
        client: typing.Optional[modal.client._Client] = None,
    ) -> list[_Secret]:
        """List named Secrets in the workspace environment as hydrated handles.

        Results are ordered newest to oldest. By default, all matching Secrets are returned.

        Added in v1.1.2.

        Args:
            max_objects: Maximum number of Secrets to return.
            created_before: Only include Secrets created before this time (datetime or ISO date string).
            environment_name: Environment to list from; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.

        Returns:
            Hydrated `Secret` objects for each named Secret in the listing.

        Examples:
            ```python
            secrets = modal.Secret.objects.list()
            print([s.name for s in secrets])
            ```

            Secrets will be retrieved from the active environment, or another one can be specified:

            ```python notest
            dev_secrets = modal.Secret.objects.list(environment_name="dev")
            ```

            By default, all named Secrets are returned, newest to oldest. It's also possible to limit the
            number of results and to filter by creation date:

            ```python
            secrets = modal.Secret.objects.list(max_objects=10, created_before="2025-01-01")
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
        """Delete a named Secret entirely.

        Deletion is irreversible and affects any Apps using this Secret.

        Added in v1.1.2.

        Args:
            name: Name of the Secret to delete.
            allow_missing: If True, do nothing when the Secret does not exist.
            environment_name: Environment to delete from; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.

        Examples:
            ```python notest
            await modal.Secret.objects.delete("my-secret")
            ```

            Secrets will be deleted from the active environment, or another one can be specified:

            ```python notest
            await modal.Secret.objects.delete("my-secret", environment_name="dev")
            ```
        """
        ...

class SecretManager:
    """Namespace with methods for managing named Secret objects."""
    def __init__(self, /, *args, **kwargs):
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    class __create_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            name: str,
            env_dict: dict[str, str],
            *,
            allow_existing: bool = False,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> None:
            """Create a new named Secret in the workspace environment.

            This does not return a local handle; use `modal.Secret.from_name` to look up the Secret after creation.

            Added in v1.1.2.

            Args:
                name: Name for the new Secret.
                env_dict: Environment variable keys and values stored in the Secret.
                allow_existing: If True, do nothing when a Secret with this name already exists (existing values are kept).
                environment_name: Environment to create in; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Examples:
                ```python notest
                contents = {"MY_KEY": "my-value", "MY_OTHER_KEY": "my-other-value"}
                modal.Secret.objects.create("my-secret", contents)
                ```

                Secrets will be created in the active environment, or another one can be specified:

                ```python notest
                modal.Secret.objects.create("my-secret", contents, environment_name="dev")
                ```

                By default, an error will be raised if the Secret already exists, but passing
                `allow_existing=True` will make the creation attempt a no-op in this case.
                If the `env_dict` data differs from the existing Secret, it will be ignored.

                ```python notest
                modal.Secret.objects.create("my-secret", contents, allow_existing=True)
                ```

                Note that this method does not return a local instance of the Secret. You can use
                `modal.Secret.from_name` to perform a lookup after creation.
            """
            ...

        async def aio(
            self,
            /,
            name: str,
            env_dict: dict[str, str],
            *,
            allow_existing: bool = False,
            environment_name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
        ) -> None:
            """Create a new named Secret in the workspace environment.

            This does not return a local handle; use `modal.Secret.from_name` to look up the Secret after creation.

            Added in v1.1.2.

            Args:
                name: Name for the new Secret.
                env_dict: Environment variable keys and values stored in the Secret.
                allow_existing: If True, do nothing when a Secret with this name already exists (existing values are kept).
                environment_name: Environment to create in; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Examples:
                ```python notest
                contents = {"MY_KEY": "my-value", "MY_OTHER_KEY": "my-other-value"}
                modal.Secret.objects.create("my-secret", contents)
                ```

                Secrets will be created in the active environment, or another one can be specified:

                ```python notest
                modal.Secret.objects.create("my-secret", contents, environment_name="dev")
                ```

                By default, an error will be raised if the Secret already exists, but passing
                `allow_existing=True` will make the creation attempt a no-op in this case.
                If the `env_dict` data differs from the existing Secret, it will be ignored.

                ```python notest
                modal.Secret.objects.create("my-secret", contents, allow_existing=True)
                ```

                Note that this method does not return a local instance of the Secret. You can use
                `modal.Secret.from_name` to perform a lookup after creation.
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
        ) -> list[Secret]:
            """List named Secrets in the workspace environment as hydrated handles.

            Results are ordered newest to oldest. By default, all matching Secrets are returned.

            Added in v1.1.2.

            Args:
                max_objects: Maximum number of Secrets to return.
                created_before: Only include Secrets created before this time (datetime or ISO date string).
                environment_name: Environment to list from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                Hydrated `Secret` objects for each named Secret in the listing.

            Examples:
                ```python
                secrets = modal.Secret.objects.list()
                print([s.name for s in secrets])
                ```

                Secrets will be retrieved from the active environment, or another one can be specified:

                ```python notest
                dev_secrets = modal.Secret.objects.list(environment_name="dev")
                ```

                By default, all named Secrets are returned, newest to oldest. It's also possible to limit the
                number of results and to filter by creation date:

                ```python
                secrets = modal.Secret.objects.list(max_objects=10, created_before="2025-01-01")
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
        ) -> list[Secret]:
            """List named Secrets in the workspace environment as hydrated handles.

            Results are ordered newest to oldest. By default, all matching Secrets are returned.

            Added in v1.1.2.

            Args:
                max_objects: Maximum number of Secrets to return.
                created_before: Only include Secrets created before this time (datetime or ISO date string).
                environment_name: Environment to list from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                Hydrated `Secret` objects for each named Secret in the listing.

            Examples:
                ```python
                secrets = modal.Secret.objects.list()
                print([s.name for s in secrets])
                ```

                Secrets will be retrieved from the active environment, or another one can be specified:

                ```python notest
                dev_secrets = modal.Secret.objects.list(environment_name="dev")
                ```

                By default, all named Secrets are returned, newest to oldest. It's also possible to limit the
                number of results and to filter by creation date:

                ```python
                secrets = modal.Secret.objects.list(max_objects=10, created_before="2025-01-01")
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
            """Delete a named Secret entirely.

            Deletion is irreversible and affects any Apps using this Secret.

            Added in v1.1.2.

            Args:
                name: Name of the Secret to delete.
                allow_missing: If True, do nothing when the Secret does not exist.
                environment_name: Environment to delete from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Examples:
                ```python notest
                await modal.Secret.objects.delete("my-secret")
                ```

                Secrets will be deleted from the active environment, or another one can be specified:

                ```python notest
                await modal.Secret.objects.delete("my-secret", environment_name="dev")
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
            """Delete a named Secret entirely.

            Deletion is irreversible and affects any Apps using this Secret.

            Added in v1.1.2.

            Args:
                name: Name of the Secret to delete.
                allow_missing: If True, do nothing when the Secret does not exist.
                environment_name: Environment to delete from; defaults to the active environment.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Examples:
                ```python notest
                await modal.Secret.objects.delete("my-secret")
                ```

                Secrets will be deleted from the active environment, or another one can be specified:

                ```python notest
                await modal.Secret.objects.delete("my-secret", environment_name="dev")
                ```
            """
            ...

    delete: __delete_spec

async def _load_from_env_dict(
    instance: _Secret, load_context: modal._load_context.LoadContext, env_dict: dict[str, str]
):
    """helper method for loaders .from_dict and .from_dotenv etc."""
    ...

class _Secret(modal._object._Object):
    """Secrets provide a dictionary of environment variables for images.

    Secrets are a secure way to add credentials and other sensitive information
    to the containers your functions run in. You can create and edit secrets on
    [the dashboard](https://modal.com/secrets), or programmatically from Python code.

    See [the secrets guide page](https://modal.com/docs/guide/secrets) for more information.
    """

    _metadata: typing.Optional[modal_proto.api_pb2.SecretMetadata]
    _load_env_dict: typing.Optional[collections.abc.Callable[[], dict[str, str]]]

    @synchronicity.classproperty
    @classmethod
    def objects(cls) -> _SecretManager: ...
    @property
    def name(self) -> typing.Optional[str]: ...
    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self) -> modal_proto.api_pb2.SecretMetadata: ...
    @property
    def _is_ephemeral(self) -> bool:
        """Whether this Secret is backed by a locally-resolvable env dict rather than a named deployment.

        True for Secrets created via `from_dict`, `from_dotenv`, or `from_local_environ`, whose contents
        can be resolved locally and inlined; False for `from_name` references that must be resolved server-side.
        """
        ...

    @staticmethod
    def from_dict(env_dict: dict[str, typing.Optional[str]] = {}) -> _Secret:
        """Create a Secret from a dictionary of environment variable names to string values.

        Values may be ``None``; those keys are omitted from the Secret.

        Args:
            env_dict: Mapping of variable names to values (or ``None`` to skip a key).

        Returns:
            A lazy `Secret` handle backed by the given key-value pairs.

        Examples:
            ```python
            @app.function(secrets=[modal.Secret.from_dict({"FOO": "bar"})])
            def run():
                print(os.environ["FOO"])
            ```
        """
        ...

    @staticmethod
    def from_local_environ(env_keys: list[str]) -> _Secret:
        """Build a Secret from the current process environment (local runs only).

        In remote execution, returns an empty Secret.

        Args:
            env_keys: Names of environment variables to copy into the Secret.

        Returns:
            A `Secret` containing the resolved variables (or empty when not local).
        """
        ...

    @staticmethod
    def from_dotenv(path=None, *, filename=".env", client: typing.Optional[modal.client._Client] = None) -> _Secret:
        """Load environment variables from a `.env` file into a Secret.

        With no `path`, searches from the current working directory (not the caller's file path).
        With `path` set, walks upward from that file or directory to find `filename`.

        Args:
            path: File or directory to search from; omit to search from the process cwd.
            filename: Name of the env file to find (default ``.env``).
            client: Modal client used when hydrating the Secret.

        Examples:
            ```python
            @app.function(secrets=[modal.Secret.from_dotenv(__file__)])
            def run():
                print(os.environ["USERNAME"])  # Assumes USERNAME is defined in your .env file
            ```

            ```python
            @app.function(secrets=[modal.Secret.from_dotenv(filename=".env-dev")])
            def run():
                ...
            ```

        Returns:
            A lazy `Secret` handle whose values are loaded from the resolved `.env` file.
        """
        ...

    @classmethod
    def _from_load_env_dict(
        cls, load_env_dict: collections.abc.Callable[[], dict[str, str]], *args, **kwargs
    ) -> _Secret: ...
    @staticmethod
    def from_name(
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        required_keys: list[str] = [],
        client: typing.Optional[modal.client._Client] = None,
    ) -> _Secret:
        """Reference a deployed Secret by name.

        Hydration is lazy until the Secret is used.

        Args:
            name: Deployment name of the Secret.
            environment_name: Environment to resolve the name in; defaults to the active environment.
            required_keys: If non-empty, the server asserts these keys exist on the Secret.
            client: Modal client to use for loading; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Secret` handle (possibly not yet hydrated).

        Examples:
            ```python
            secret = modal.Secret.from_name("my-secret")

            @app.function(secrets=[secret])
            def run():
                ...
            ```
        """
        ...

    @staticmethod
    async def _create_deployed(
        deployment_name: str,
        env_dict: dict[str, str],
        client: typing.Optional[modal.client._Client] = None,
        environment_name: typing.Optional[str] = None,
        overwrite: bool = False,
    ) -> str:
        """mdmd:hidden"""
        ...

    async def info(self) -> modal.types.SecretInfo:
        """Return information about the Secret object."""
        ...

    async def update(self, env_dict: dict[str, str]) -> None:
        """Update this Secret, adding or overwriting key-value pairs.

        Like dict.update(), this merges `env_dict` into the existing Secret.
        Keys not mentioned in `env_dict` are left unchanged.
        """
        ...

def _split_env_dict_and_resolvable_secrets(secrets: list[_Secret]) -> tuple[dict[str, str], list[_Secret]]:
    """Split secrets into secrets that can be resolved locally and secrets that are remote.

    Locally resolvable secrets include: `Secret.from_dict`, `Secret.from_dotenv`
    Remote secrets include: `Secret.from_name`
    """
    ...

class Secret(modal.object.Object):
    """Secrets provide a dictionary of environment variables for images.

    Secrets are a secure way to add credentials and other sensitive information
    to the containers your functions run in. You can create and edit secrets on
    [the dashboard](https://modal.com/secrets), or programmatically from Python code.

    See [the secrets guide page](https://modal.com/docs/guide/secrets) for more information.
    """

    _metadata: typing.Optional[modal_proto.api_pb2.SecretMetadata]
    _load_env_dict: typing.Optional[collections.abc.Callable[[], dict[str, str]]]

    def __init__(self, *args, **kwargs):
        """mdmd:hidden"""
        ...

    @synchronicity.classproperty
    @classmethod
    def objects(cls) -> SecretManager: ...
    @property
    def name(self) -> typing.Optional[str]: ...
    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self) -> modal_proto.api_pb2.SecretMetadata: ...
    @property
    def _is_ephemeral(self) -> bool:
        """Whether this Secret is backed by a locally-resolvable env dict rather than a named deployment.

        True for Secrets created via `from_dict`, `from_dotenv`, or `from_local_environ`, whose contents
        can be resolved locally and inlined; False for `from_name` references that must be resolved server-side.
        """
        ...

    @staticmethod
    def from_dict(env_dict: dict[str, typing.Optional[str]] = {}) -> Secret:
        """Create a Secret from a dictionary of environment variable names to string values.

        Values may be ``None``; those keys are omitted from the Secret.

        Args:
            env_dict: Mapping of variable names to values (or ``None`` to skip a key).

        Returns:
            A lazy `Secret` handle backed by the given key-value pairs.

        Examples:
            ```python
            @app.function(secrets=[modal.Secret.from_dict({"FOO": "bar"})])
            def run():
                print(os.environ["FOO"])
            ```
        """
        ...

    @staticmethod
    def from_local_environ(env_keys: list[str]) -> Secret:
        """Build a Secret from the current process environment (local runs only).

        In remote execution, returns an empty Secret.

        Args:
            env_keys: Names of environment variables to copy into the Secret.

        Returns:
            A `Secret` containing the resolved variables (or empty when not local).
        """
        ...

    @staticmethod
    def from_dotenv(path=None, *, filename=".env", client: typing.Optional[modal.client.Client] = None) -> Secret:
        """Load environment variables from a `.env` file into a Secret.

        With no `path`, searches from the current working directory (not the caller's file path).
        With `path` set, walks upward from that file or directory to find `filename`.

        Args:
            path: File or directory to search from; omit to search from the process cwd.
            filename: Name of the env file to find (default ``.env``).
            client: Modal client used when hydrating the Secret.

        Examples:
            ```python
            @app.function(secrets=[modal.Secret.from_dotenv(__file__)])
            def run():
                print(os.environ["USERNAME"])  # Assumes USERNAME is defined in your .env file
            ```

            ```python
            @app.function(secrets=[modal.Secret.from_dotenv(filename=".env-dev")])
            def run():
                ...
            ```

        Returns:
            A lazy `Secret` handle whose values are loaded from the resolved `.env` file.
        """
        ...

    @classmethod
    def _from_load_env_dict(
        cls, load_env_dict: collections.abc.Callable[[], dict[str, str]], *args, **kwargs
    ) -> Secret: ...
    @staticmethod
    def from_name(
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        required_keys: list[str] = [],
        client: typing.Optional[modal.client.Client] = None,
    ) -> Secret:
        """Reference a deployed Secret by name.

        Hydration is lazy until the Secret is used.

        Args:
            name: Deployment name of the Secret.
            environment_name: Environment to resolve the name in; defaults to the active environment.
            required_keys: If non-empty, the server asserts these keys exist on the Secret.
            client: Modal client to use for loading; defaults to `Client.from_env()` when omitted.

        Returns:
            A `Secret` handle (possibly not yet hydrated).

        Examples:
            ```python
            secret = modal.Secret.from_name("my-secret")

            @app.function(secrets=[secret])
            def run():
                ...
            ```
        """
        ...

    class ___create_deployed_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            deployment_name: str,
            env_dict: dict[str, str],
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            overwrite: bool = False,
        ) -> str:
            """mdmd:hidden"""
            ...

        async def aio(
            self,
            /,
            deployment_name: str,
            env_dict: dict[str, str],
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            overwrite: bool = False,
        ) -> str:
            """mdmd:hidden"""
            ...

    _create_deployed: typing.ClassVar[___create_deployed_spec]

    class __info_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal.types.SecretInfo:
            """Return information about the Secret object."""
            ...

        async def aio(self, /) -> modal.types.SecretInfo:
            """Return information about the Secret object."""
            ...

    info: __info_spec

    class __update_spec(typing_extensions.Protocol):
        def __call__(self, /, env_dict: dict[str, str]) -> None:
            """Update this Secret, adding or overwriting key-value pairs.

            Like dict.update(), this merges `env_dict` into the existing Secret.
            Keys not mentioned in `env_dict` are left unchanged.
            """
            ...

        async def aio(self, /, env_dict: dict[str, str]) -> None:
            """Update this Secret, adding or overwriting key-value pairs.

            Like dict.update(), this merges `env_dict` into the existing Secret.
            Keys not mentioned in `env_dict` are left unchanged.
            """
            ...

    update: __update_spec
