import collections.abc
import modal._functions
import modal._image
import modal._load_context
import modal._partial_function
import modal._server
import modal._utils.function_utils
import modal.client
import modal.cloud_bucket_mount
import modal.cls
import modal.functions
import modal.image
import modal.network_file_system
import modal.partial_function
import modal.proxy
import modal.retries
import modal.running_app
import modal.schedule
import modal.secret
import modal.server
import modal.volume
import pathlib
import synchronicity.combined_types
import typing
import typing_extensions

class _LocalEntrypoint:
    _info: modal._utils.function_utils.FunctionInfo
    _app: _App

    def __init__(self, info: modal._utils.function_utils.FunctionInfo, app: _App) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __call__(self, *args: typing.Any, **kwargs: typing.Any) -> typing.Any:
        """Call self as a function."""
        ...

    @property
    def info(self) -> modal._utils.function_utils.FunctionInfo: ...
    @property
    def app(self) -> _App: ...

class LocalEntrypoint:
    _info: modal._utils.function_utils.FunctionInfo
    _app: App

    def __init__(self, info: modal._utils.function_utils.FunctionInfo, app: App) -> None: ...
    def __call__(self, *args: typing.Any, **kwargs: typing.Any) -> typing.Any: ...
    @property
    def info(self) -> modal._utils.function_utils.FunctionInfo: ...
    @property
    def app(self) -> App: ...

def check_sequence(items: typing.Sequence[typing.Any], item_type: type[typing.Any], error_msg: str) -> None: ...

CLS_T = typing.TypeVar("CLS_T", bound="type[typing.Any]")

P = typing.ParamSpec("P")

ReturnType = typing.TypeVar("ReturnType")

OriginalReturnType = typing.TypeVar("OriginalReturnType")

class _FunctionDecoratorType:
    @typing.overload
    def __call__(
        self, func: modal.partial_function.PartialFunction[P, ReturnType, OriginalReturnType]
    ) -> modal.functions.Function[P, ReturnType, OriginalReturnType]: ...
    @typing.overload
    def __call__(
        self, func: collections.abc.Callable[P, collections.abc.Coroutine[typing.Any, typing.Any, ReturnType]]
    ) -> modal.functions.Function[P, ReturnType, collections.abc.Coroutine[typing.Any, typing.Any, ReturnType]]: ...
    @typing.overload
    def __call__(
        self, func: collections.abc.Callable[P, ReturnType]
    ) -> modal.functions.Function[P, ReturnType, ReturnType]: ...

class _LocalAppState:
    """All state for apps that's part of the local/definition state"""

    functions: dict[str, modal._functions._Function]
    classes: dict[str, modal.cls._Cls]
    image_default: typing.Optional[modal._image._Image]
    web_endpoints: list[str]
    local_entrypoints: dict[str, _LocalEntrypoint]
    tags: dict[str, str]
    include_source_default: bool
    secrets_default: collections.abc.Sequence[modal.secret._Secret]
    volumes_default: dict[typing.Union[str, pathlib.PurePosixPath], modal.volume._Volume]

    def __init__(
        self,
        functions: dict[str, modal._functions._Function],
        classes: dict[str, modal.cls._Cls],
        image_default: typing.Optional[modal._image._Image],
        web_endpoints: list[str],
        local_entrypoints: dict[str, _LocalEntrypoint],
        tags: dict[str, str],
        include_source_default: bool,
        secrets_default: collections.abc.Sequence[modal.secret._Secret],
        volumes_default: dict[typing.Union[str, pathlib.PurePosixPath], modal.volume._Volume],
    ) -> None:
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def __repr__(self):
        """Return repr(self)."""
        ...

    def __eq__(self, other):
        """Return self==value."""
        ...

class _App:
    """A Modal App is a group of functions and classes that are deployed together.

    The app serves at least three purposes:

    * A unit of deployment for functions and classes.
    * Syncing of identities of (primarily) functions and classes across processes
      (your local Python interpreter and every Modal container active in your application).
    * Manage log collection for everything that happens inside your code.

    **Registering functions with an app**

    The most common way to explicitly register an Object with an app is through the
    `@app.function()` decorator. It both registers the annotated function itself and
    other passed objects, like schedules and secrets, with the app:

    ```python
    import modal

    app = modal.App()

    @app.function(
        secrets=[modal.Secret.from_name("some_secret")],
        schedule=modal.Period(days=1),
    )
    def foo():
        pass
    ```

    In this example, the secret and schedule are registered with the app.
    """

    _all_apps: typing.ClassVar[dict[typing.Optional[str], list[_App]]]
    _container_app: typing.ClassVar[typing.Optional[_App]]
    _name: typing.Optional[str]
    _description: typing.Optional[str]
    _local_state_attr: typing.Optional[_LocalAppState]
    _app_id: typing.Optional[str]
    _running_app: typing.Optional[modal.running_app.RunningApp]
    _client: typing.Optional[modal.client._Client]
    _root_load_context: modal._load_context.LoadContext

    @property
    def _local_state(self) -> _LocalAppState:
        """For internal use only. Do not use this property directly."""
        ...

    def __init__(
        self,
        name: typing.Optional[str] = None,
        *,
        tags: typing.Optional[dict[str, str]] = None,
        image: typing.Optional[modal._image._Image] = None,
        secrets: collections.abc.Sequence[modal.secret._Secret] = [],
        volumes: dict[typing.Union[str, pathlib.PurePosixPath], modal.volume._Volume] = {},
        include_source: bool = True,
    ) -> None:
        """Construct a new app, optionally with default image, mounts, secrets, or volumes.

        Args:
            name: Optional app name used for registration and lookup.
            tags: Additional metadata to set on the App.
            image: Default image for the App (otherwise defaults to `modal.Image.debian_slim()`).
            secrets: Secrets to add for all Functions in the App.
            volumes: Volume mounts to use for all Functions.
            include_source:
                Default for whether Function source files are added to the Modal container (per-function override
                possible).


        Examples:
            ```python notest
            image = modal.Image.debian_slim().pip_install(...)
            secret = modal.Secret.from_name("my-secret")
            volume = modal.Volume.from_name("my-data")
            app = modal.App(image=image, secrets=[secret], volumes={"/mnt/data": volume})
            ```
        """
        ...

    @property
    def name(self) -> typing.Optional[str]:
        """The user-provided name of the App.

        Returns:
            The configured app name, if any.
        """
        ...

    @property
    def is_interactive(self) -> bool:
        """mdmd:hidden
        Whether the current app for the app is running in interactive mode.

        Note: this method will likely be deprecated in the future.
        """
        ...

    @property
    def app_id(self) -> typing.Optional[str]:
        """Return the app_id of a running or stopped app.

        Returns:
            The app ID when the app has been deployed or run, otherwise None.
        """
        ...

    @property
    def description(self) -> typing.Optional[str]:
        """The App's `name`, if available, or a fallback descriptive identifier.

        Returns:
            Human-readable description string for the app.
        """
        ...

    @staticmethod
    async def lookup(
        name: str,
        *,
        client: typing.Optional[modal.client._Client] = None,
        environment_name: typing.Optional[str] = None,
        create_if_missing: bool = False,
    ) -> _App:
        """Look up an App with a given name, creating a new App if necessary.

        Note that Apps created through this method will be in a deployed state,
        but they will not have any associated Functions or Classes. This method
        is mainly useful for creating an App to associate with a Sandbox.

        Args:
            name: App name to resolve or create.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.
            environment_name: Optional environment name; defaults to the configured environment.
            create_if_missing: If True, create the app when it does not already exist.

        Returns:
            An `App` handle tied to the deployed app record.

        Examples:
            ```python
            app = modal.App.lookup("my-app", create_if_missing=True)
            modal.Sandbox.create("echo", "hi", app=app)
            ```
        """
        ...

    async def get_dashboard_url(self) -> str:
        """Get the dashboard URL for the App.

        Returns:
            The dashboard URL for the App.

        Examples:
            ```python
            app = modal.App.lookup("my-app")
            print(app.get_dashboard_url())
            ```
        """
        ...

    def set_description(self, description: str):
        """mdmd:hidden
        Set the description of the App before it starts running.

        Note: we don't recommend using the method and may deprecate it in the future.
        """
        ...

    def _validate_blueprint_value(self, key: str, value: typing.Any): ...
    @property
    def image(self) -> modal._image._Image:
        """mdmd:hidden
        Retrieve the Image that will be used as the default for any Functions registered to the App.

        Note: This property is only relevant in the build phase and won't be populated on a deployed
        App that is retrieved via `modal.App.lookup`. It is likely to be deprecated in the future.
        """
        ...

    @image.setter
    def image(self, value):
        """mdmd:hidden"""
        ...

    def _uncreate_all_objects(self): ...
    def _set_local_app(
        self, client: modal.client._Client, running_app: modal.running_app.RunningApp
    ) -> typing.AsyncContextManager[None]: ...
    def run(
        self,
        *,
        name: typing.Optional[str] = None,
        client: typing.Optional[modal.client._Client] = None,
        detach: bool = False,
        interactive: bool = False,
        environment_name: typing.Optional[str] = None,
    ) -> typing.AsyncContextManager[_App]:
        """Context manager that runs an ephemeral app on Modal.

        Use this as the main entry point for your Modal application. All calls
        to Modal Functions should be made within the scope of this context
        manager, and they will correspond to the current App.

        Note that you should not invoke this in global scope of a file where you have
        Modal Functions or Classes defined, since that would run the block when the Function
        or Cls is imported in your containers as well. If you want to run it as your entrypoint,
        consider protecting it with ``if __name__ == "__main__"``.

        Args:
            client: Modal client to use for the run session.
            detach: Whether to detach after starting the app.
            interactive: Whether to run in interactive mode.
            environment_name: Optional environment name; defaults to the configured environment.

        Returns:
            Async context manager yielding this `App` while it is running.

        Examples:
            ```python notest
            with app.run():
                some_modal_function.remote()
            ```

            To enable output printing (i.e., to see App logs), use `modal.enable_output()`:

            ```python notest
            with modal.enable_output():
                with app.run():
                    some_modal_function.remote()
            ```

            Note that you should not invoke this in global scope of a file where you have Modal
            Functions or Classes defined, since that would run the block when the Function or Cls
            is imported in your containers as well. If you want to run it as your entrypoint,
            consider protecting it:

            ```python
            if __name__ == "__main__":
                with app.run():
                    some_modal_function.remote()
            ```

            You can then run your script with:

            ```shell
            python app_module.py
            ```
        """
        ...

    async def deploy(
        self,
        *,
        name: typing.Optional[str] = None,
        environment_name: typing.Optional[str] = None,
        tag: str = "",
        client: typing.Optional[modal.client._Client] = None,
        strategy: str = "rolling",
    ) -> typing_extensions.Self:
        """Deploy the App so that it is available persistently.

        Deployed Apps will be available for lookup or web-based invocations until they are stopped.
        Unlike with `App.run`, this method will return as soon as the deployment completes.

        This method is a programmatic alternative to the `modal deploy` CLI command.

        Unlike with `App.run`, Function logs will not stream back to the local client after the
        App is deployed.

        Note that you should not invoke this method in global scope, as that would redeploy
        the App every time the file is imported. If you want to write a programmatic deployment
        script, protect this call so that it only runs when the file is executed directly.

        Args:
            name: Name for the deployment, overriding any set on the App.
            environment_name: Environment to deploy the App in.
            tag: Optional metadata that is specific to this deployment.
            client: Alternate client to use for communication with the server.
            strategy:
                Deployment strategy. ``rolling`` (default) shifts traffic gradually to new containers while old ones
                drain. ``recreate`` terminates all running containers as part of the deployment before new work starts.

        Returns:
            This app instance after deployment completes.

        Examples:
            ```python notest
            app = App("my-app")
            app.deploy()
            ```

            To enable output printing (i.e., to see build logs), use `modal.enable_output()`:

            ```python notest
            app = App("my-app")
            with modal.enable_output():
                app.deploy()
            ```

            Unlike with `App.run`, Function logs will not stream back to the local client after the App is deployed.

            Note that you should not invoke this method in global scope, as that would redeploy the App every time
            the file is imported. If you want to write a programmatic deployment script, protect this call so that it
            only runs when the file is executed directly. You can then run your script with:

            ```python notest
            if __name__ == "__main__":
                with modal.enable_output():
                    app.deploy()
            ```

            Then you can deploy your app with:

            ```shell
            python app_module.py
            ```
        """
        ...

    def _get_default_image(self): ...
    def _get_watch_mounts(self): ...
    def _add_function(self, function: modal._functions._Function, is_web_endpoint: bool): ...
    def _add_class(self, tag: str, cls: modal.cls._Cls): ...
    def _init_container(self, client: modal.client._Client, running_app: modal.running_app.RunningApp): ...
    @property
    def registered_functions(self) -> dict[str, modal._functions._Function]:
        """mdmd:hidden
        All modal.Function objects registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future in favor of a different
        approach for retrieving the layout of a deployed App.
        """
        ...

    @property
    def registered_classes(self) -> dict[str, modal.cls._Cls]:
        """mdmd:hidden
        All modal.Cls objects registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future in favor of a different
        approach for retrieving the layout of a deployed App.
        """
        ...

    @property
    def registered_entrypoints(self) -> dict[str, _LocalEntrypoint]:
        """mdmd:hidden
        All local CLI entrypoints registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future.
        """
        ...

    @property
    def registered_web_endpoints(self) -> list[str]:
        """mdmd:hidden
        Names of Web Functions registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future in favor of a different
        approach for retrieving the layout of a deployed App.
        """
        ...

    def local_entrypoint(
        self, _warn_parentheses_missing: typing.Any = None, *, name: typing.Optional[str] = None
    ) -> collections.abc.Callable[[collections.abc.Callable[..., typing.Any]], _LocalEntrypoint]:
        """Decorate a function to be used as a CLI entrypoint for a Modal App.

        These functions can be used to define code that runs locally to set up the app,
        and act as an entrypoint to start Modal functions from. Note that regular
        Modal functions can also be used as CLI entrypoints, but unlike `local_entrypoint`,
        those functions are executed remotely directly.

        Note that an explicit [`app.run()`](https://modal.com/docs/sdk/py/latest/App#run) is not needed, as an
        [app](https://modal.com/docs/guide/apps) is automatically created for you.

        Args:
            name: Optional name for the entrypoint; defaults to the function's qualified name.

        Returns:
            A decorator that registers the wrapped callable as a local CLI entrypoint.

        Examples:
            ```python
            @app.local_entrypoint()
            def main():
                some_modal_function.remote()
            ```

            You can call the function using `modal run` directly from the CLI:

            ```shell
            modal run app_module.py
            ```

            Note that an explicit `app.run()` is not needed, as an app is automatically created for you.

            **Multiple entrypoints**

            If you have multiple `local_entrypoint` functions, qualify the name:

            ```shell
            modal run app_module.py::app.some_other_function
            ```

            **Parsing arguments**

            If your entrypoint function take arguments with primitive types, `modal run` automatically
            parses them as CLI options. For example, the following function can be called with
            `modal run app_module.py --foo 1 --bar "hello"`:

            ```python
            @app.local_entrypoint()
            def main(foo: int, bar: str):
                some_modal_function.call(foo, bar)
            ```

            Currently, `str`, `int`, `float`, `bool`, and `datetime.datetime` are supported.
            Use `modal run app_module.py --help` for more information on usage.
        """
        ...

    def function(
        self,
        _warn_parentheses_missing=None,
        *,
        image: typing.Optional[modal._image._Image] = None,
        schedule: typing.Optional[modal.schedule.Schedule] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        gpu: typing.Union[str, list[str], None] = None,
        serialized: bool = False,
        network_file_systems: dict[
            typing.Union[str, pathlib.PurePosixPath], modal.network_file_system._NetworkFileSystem
        ] = {},
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        ephemeral_disk: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        retries: typing.Union[int, modal.retries.Retries, None] = None,
        timeout: int = 300,
        startup_timeout: typing.Optional[int] = None,
        name: typing.Optional[str] = None,
        is_generator: typing.Optional[bool] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        routing_region: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        enable_memory_snapshot: bool = False,
        block_network: bool = False,
        restrict_modal_access: bool = False,
        single_use_containers: bool = False,
        i6pn: typing.Optional[bool] = None,
        include_source: typing.Optional[bool] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        _experimental_restrict_output: bool = False,
        max_inputs: typing.Optional[int] = None,
    ) -> _FunctionDecoratorType:
        """Decorator to register a new Modal Function with this App.

        Args:
            image: The image to run as the container for the function.
            schedule: An optional Modal Schedule for the function.
            env: Environment variables to set in the container.
            secrets: Secrets to inject into the container as environment variables.
            gpu: GPU request; either a single GPU type or a list of types.
            serialized: Whether to send the function over using cloudpickle.
            network_file_systems: Mountpoints for Modal NetworkFileSystems.
            volumes: Mount points for Modal Volumes & CloudBucketMounts.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container from
                exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            ephemeral_disk: Specify, in MiB, the ephemeral disk size for the Function.
            min_containers: Minimum number of containers to keep warm, even when Function is idle.
            max_containers: Limit on the number of containers that can be concurrently running.
            buffer_containers: Number of additional idle containers to maintain under active load.
            scaledown_window: Max time (in seconds) a container can remain idle while scaling down.
            proxy: Reference to a Modal Proxy to use in front of this function.
            retries: Number of times to retry each input in case of failure.
            timeout: Maximum execution time for inputs and startup time in seconds.
            startup_timeout: Maximum startup time in seconds with higher precedence than `timeout`.
            name: Sets the Modal name of the function within the app.
            is_generator:
                Set this to True if it's a non-generator function returning a sync or async generator object.
            cloud: Cloud provider to run the function on. Possible values are aws, gcp, oci, auto.
            region: Region or regions to run the function on.
            routing_region: Region to route inputs to the function through.
            nonpreemptible: Whether to run the function on a nonpreemptible instance.
            enable_memory_snapshot: Enable memory checkpointing for faster cold starts.
            block_network: Whether to block network access.
            restrict_modal_access: Whether to allow this function access to other Modal resources.
            single_use_containers: When True, containers will shut down after handling a single input.
            i6pn: Whether to enable IPv6 container networking within the region.
            include_source:
                Whether the file or directory containing the Function's source should automatically be included in the
                container. When unset, falls back to the App-level configuration, or is otherwise True by default.
            experimental_options: Experimental options for the function.
            _experimental_restrict_output: Experimental; do not use pickle for return values.
            max_inputs: Deprecated; replaced with `single_use_containers`.

        Returns:
            A decorator that registers the wrapped callable or partial as a Modal `Function`.
        """
        ...

    @typing_extensions.dataclass_transform(
        field_specifiers=(modal.cls.parameter,),
        kw_only_default=True,
    )
    def cls(
        self,
        _warn_parentheses_missing=None,
        *,
        image: typing.Optional[modal._image._Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        gpu: typing.Union[str, list[str], None] = None,
        serialized: bool = False,
        network_file_systems: dict[
            typing.Union[str, pathlib.PurePosixPath], modal.network_file_system._NetworkFileSystem
        ] = {},
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        ephemeral_disk: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        retries: typing.Union[int, modal.retries.Retries, None] = None,
        timeout: int = 300,
        startup_timeout: typing.Optional[int] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        routing_region: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        enable_memory_snapshot: bool = False,
        block_network: bool = False,
        restrict_modal_access: bool = False,
        single_use_containers: bool = False,
        i6pn: typing.Optional[bool] = None,
        include_source: typing.Optional[bool] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        _experimental_restrict_output: bool = False,
        max_inputs: typing.Optional[int] = None,
    ) -> collections.abc.Callable[[typing.Union[CLS_T, modal._partial_function._PartialFunction]], CLS_T]:
        """Decorator to register a new Modal [Cls](https://modal.com/docs/sdk/py/latest/Cls) with this App.

        Args:
            image: The image to run as the container for the class service.
            env: Environment variables to set in the container.
            secrets: Secrets to inject into the container as environment variables.
            gpu: GPU request; either a single GPU type or a list of types.
            serialized: Whether to send the class over using cloudpickle.
            network_file_systems: Mountpoints for Modal NetworkFileSystems.
            volumes: Mount points for Modal Volumes & CloudBucketMounts.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container from
                exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            ephemeral_disk: Specify, in MiB, the ephemeral disk size for the Function.
            min_containers: Minimum number of containers to keep warm, even when Function is idle.
            max_containers: Limit on the number of containers that can be concurrently running.
            buffer_containers: Number of additional idle containers to maintain under active load.
            scaledown_window: Max time (in seconds) a container can remain idle while scaling down.
            proxy: Reference to a Modal Proxy to use in front of this function.
            retries: Number of times to retry each input in case of failure.
            timeout: Maximum execution time for inputs and startup time in seconds.
            startup_timeout: Maximum startup time in seconds with higher precedence than `timeout`.
            cloud: Cloud provider to run the function on. Possible values are aws, gcp, oci, auto.
            region: Region or regions to run the function on.
            routing_region: Region to route inputs to the function through.
            nonpreemptible: Whether to run the function on a non-preemptible instance.
            enable_memory_snapshot: Enable memory checkpointing for faster cold starts.
            block_network: Whether to block network access.
            restrict_modal_access: Whether to allow this class access to other Modal resources.
            single_use_containers: When True, containers will shut down after handling a single input.
            i6pn: Whether to enable IPv6 container networking within the region.
            include_source: When ``False``, don't automatically add the App source to the container.
            experimental_options: Experimental options for the class service.
            _experimental_restrict_output: Experimental; do not use pickle for return values.
            max_inputs: Deprecated; replaced with `single_use_containers`.

        Returns:
            A decorator that registers the wrapped class or partial as a Modal `Cls`.
        """
        ...

    def server(
        self,
        _warn_parentheses_missing=None,
        *,
        image: typing.Optional[modal._image._Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret._Secret]] = None,
        gpu: typing.Union[str, list[str], None] = None,
        serialized: bool = False,
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume._Volume, modal.cloud_bucket_mount._CloudBucketMount],
        ] = {},
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        ephemeral_disk: typing.Optional[int] = None,
        target_concurrency: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaleup_window: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        startup_timeout: int = 30,
        name: typing.Optional[str] = None,
        port: int = 8000,
        unauthenticated: bool = False,
        h2_enabled: bool = False,
        exit_grace_period: int = 0,
        routing_region: str = "us-east",
        compute_region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cloud: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        proxy: typing.Optional[modal.proxy._Proxy] = None,
        i6pn: typing.Optional[bool] = None,
        enable_memory_snapshot: bool = False,
        include_source: typing.Optional[bool] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
    ) -> collections.abc.Callable[
        [typing.Union[CLS_T, modal._partial_function._PartialFunction]], modal._server._Server
    ]:
        """Decorator to register a new Modal Server with this App.

        Servers run HTTP servers that are started in a `@modal.enter()` method.
        Unlike `@app.cls()`, servers only expose HTTP endpoints and do not
        support `.remote()` method calls.

        See the [guide](https://modal.com/docs/guide/servers) for more information.

        Args:
            image: The image to run as the container for the server.
            env: Environment variables to set in the container.
            secrets: Secrets to inject into the container as environment variables.
            gpu: GPU request; either a single GPU type or a list of types.
            serialized: Whether to send the server class over using cloudpickle.
            volumes: Mount points for Modal Volumes & CloudBucketMounts.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container
                from exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            ephemeral_disk: Specify, in MiB, the ephemeral disk size for the server.
            target_concurrency: Target concurrency for the server; 0 disables autoscaling.
            min_containers: Minimum number of containers to keep running regardless of demand.
            max_containers: Limit on the number of containers that can be concurrently running.
            buffer_containers: Extra containers to scale up beyond current demand.
            scaleup_window: Seconds of sustained demand required before scaling up new containers.
            scaledown_window: Maximum duration (in seconds) idle containers wait before scaling down.
            startup_timeout: Maximum container startup time in seconds.
            name: Sets the Modal name of the function within the app, defaults to class name.
            port: Port the HTTP server listens on.
            unauthenticated: Whether the endpoint requires proxy authentication; required by default.
            h2_enabled: Enable HTTP/2.
            exit_grace_period: Grace period for in-flight requests on shutdown.
            routing_region: Region to route Server requests through.
            compute_region: Region(s) where containers can be scheduled.
            cloud: Cloud provider (aws, gcp, oci, auto).
            nonpreemptible: Whether to use non-preemptible instances.
            proxy: Modal Proxy to use in front of this server.
            i6pn: Enable IPv6 container networking.
            enable_memory_snapshot: Enable memory checkpointing.
            include_source: Whether to add source to container.
            experimental_options: Experimental options.

        Examples:
            ```python
            @app.server(port=8000, routing_region="us-east")
            class MyServer:
                @modal.enter()
                def start(self):
                    self.proc = subprocess.Popen(["python3", "-m", "http.server", "8000"])

                @modal.exit()
                def stop(self):
                    self.proc.terminate()
            ```
        """
        ...

    def include(self, /, other_app: _App, inherit_tags: bool = True) -> typing_extensions.Self:
        """Include another App's objects in this one.

        Useful for splitting up Modal Apps across different self-contained files.

        When `inherit_tags=True` any tags set on the other App will be inherited by this App
        (with this App's tags taking precedence in the case of conflicts).

        Args:
            other_app: App whose registered functions and classes are merged into this app.
            inherit_tags: If True, merge tags from `other_app` into this app (this app wins on conflicts).

        Returns:
            This app instance for chaining.

        Examples:
            ```python
            app_a = modal.App("a")
            @app_a.function()
            def foo():
                ...

            app_b = modal.App("b")
            @app_b.function()
            def bar():
                ...

            app_a.include(app_b)

            @app_a.local_entrypoint()
            def main():
                # use function declared on the included app
                bar.remote()
            ```
        """
        ...

    async def set_tags(
        self, tags: collections.abc.Mapping[str, str], *, client: typing.Optional[modal.client._Client] = None
    ) -> None:
        """Attach key-value metadata to the App.

        Tag metadata can be used to add organization-specific context to the App and can be
        included in billing reports and other informational APIs. Tags can also be set in
        the App constructor.

        Any tags set on the App before calling this method will be removed if they are not
        included in the argument (i.e., this method does not have `.update()` semantics).

        Args:
            tags: Complete tag set to store on the app (replaces previous tags).
            client: Modal client to use for the RPC.
        """
        ...

    async def get_tags(self, *, client: typing.Optional[modal.client._Client] = None) -> dict[str, str]:
        """Get the tags that are currently attached to the App.

        Args:
            client: Modal client to use for the RPC.

        Returns:
            Tags as a map from key to value.
        """
        ...

    def _logs(self, client: typing.Optional[modal.client._Client] = None) -> collections.abc.AsyncGenerator[str, None]:
        """Stream logs from the app.

        This method is considered private and its interface may change - use at your own risk!
        """
        ...

    @classmethod
    def _get_container_app(cls) -> typing.Optional[_App]:
        """Returns the `App` running inside a container.

        This will return `None` outside of a Modal container.
        """
        ...

    @classmethod
    def _reset_container_app(cls):
        """Only used for tests."""
        ...

SUPERSELF = typing.TypeVar("SUPERSELF", covariant=True)

class App:
    """A Modal App is a group of functions and classes that are deployed together.

    The app serves at least three purposes:

    * A unit of deployment for functions and classes.
    * Syncing of identities of (primarily) functions and classes across processes
      (your local Python interpreter and every Modal container active in your application).
    * Manage log collection for everything that happens inside your code.

    **Registering functions with an app**

    The most common way to explicitly register an Object with an app is through the
    `@app.function()` decorator. It both registers the annotated function itself and
    other passed objects, like schedules and secrets, with the app:

    ```python
    import modal

    app = modal.App()

    @app.function(
        secrets=[modal.Secret.from_name("some_secret")],
        schedule=modal.Period(days=1),
    )
    def foo():
        pass
    ```

    In this example, the secret and schedule are registered with the app.
    """

    _all_apps: typing.ClassVar[dict[typing.Optional[str], list[App]]]
    _container_app: typing.ClassVar[typing.Optional[App]]
    _name: typing.Optional[str]
    _description: typing.Optional[str]
    _local_state_attr: typing.Optional[_LocalAppState]
    _app_id: typing.Optional[str]
    _running_app: typing.Optional[modal.running_app.RunningApp]
    _client: typing.Optional[modal.client.Client]
    _root_load_context: modal._load_context.LoadContext

    def __init__(
        self,
        name: typing.Optional[str] = None,
        *,
        tags: typing.Optional[dict[str, str]] = None,
        image: typing.Optional[modal.image.Image] = None,
        secrets: collections.abc.Sequence[modal.secret.Secret] = [],
        volumes: dict[typing.Union[str, pathlib.PurePosixPath], modal.volume.Volume] = {},
        include_source: bool = True,
    ) -> None:
        """Construct a new app, optionally with default image, mounts, secrets, or volumes.

        Args:
            name: Optional app name used for registration and lookup.
            tags: Additional metadata to set on the App.
            image: Default image for the App (otherwise defaults to `modal.Image.debian_slim()`).
            secrets: Secrets to add for all Functions in the App.
            volumes: Volume mounts to use for all Functions.
            include_source:
                Default for whether Function source files are added to the Modal container (per-function override
                possible).


        Examples:
            ```python notest
            image = modal.Image.debian_slim().pip_install(...)
            secret = modal.Secret.from_name("my-secret")
            volume = modal.Volume.from_name("my-data")
            app = modal.App(image=image, secrets=[secret], volumes={"/mnt/data": volume})
            ```
        """
        ...

    @property
    def _local_state(self) -> _LocalAppState:
        """For internal use only. Do not use this property directly."""
        ...

    @property
    def name(self) -> typing.Optional[str]:
        """The user-provided name of the App.

        Returns:
            The configured app name, if any.
        """
        ...

    @property
    def is_interactive(self) -> bool:
        """mdmd:hidden
        Whether the current app for the app is running in interactive mode.

        Note: this method will likely be deprecated in the future.
        """
        ...

    @property
    def app_id(self) -> typing.Optional[str]:
        """Return the app_id of a running or stopped app.

        Returns:
            The app ID when the app has been deployed or run, otherwise None.
        """
        ...

    @property
    def description(self) -> typing.Optional[str]:
        """The App's `name`, if available, or a fallback descriptive identifier.

        Returns:
            Human-readable description string for the app.
        """
        ...

    class __lookup_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            name: str,
            *,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            create_if_missing: bool = False,
        ) -> App:
            """Look up an App with a given name, creating a new App if necessary.

            Note that Apps created through this method will be in a deployed state,
            but they will not have any associated Functions or Classes. This method
            is mainly useful for creating an App to associate with a Sandbox.

            Args:
                name: App name to resolve or create.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.
                environment_name: Optional environment name; defaults to the configured environment.
                create_if_missing: If True, create the app when it does not already exist.

            Returns:
                An `App` handle tied to the deployed app record.

            Examples:
                ```python
                app = modal.App.lookup("my-app", create_if_missing=True)
                modal.Sandbox.create("echo", "hi", app=app)
                ```
            """
            ...

        async def aio(
            self,
            /,
            name: str,
            *,
            client: typing.Optional[modal.client.Client] = None,
            environment_name: typing.Optional[str] = None,
            create_if_missing: bool = False,
        ) -> App:
            """Look up an App with a given name, creating a new App if necessary.

            Note that Apps created through this method will be in a deployed state,
            but they will not have any associated Functions or Classes. This method
            is mainly useful for creating an App to associate with a Sandbox.

            Args:
                name: App name to resolve or create.
                client: Modal client to use; defaults to `Client.from_env()` when omitted.
                environment_name: Optional environment name; defaults to the configured environment.
                create_if_missing: If True, create the app when it does not already exist.

            Returns:
                An `App` handle tied to the deployed app record.

            Examples:
                ```python
                app = modal.App.lookup("my-app", create_if_missing=True)
                modal.Sandbox.create("echo", "hi", app=app)
                ```
            """
            ...

    lookup: typing.ClassVar[__lookup_spec]

    class __get_dashboard_url_spec(typing_extensions.Protocol):
        def __call__(self, /) -> str:
            """Get the dashboard URL for the App.

            Returns:
                The dashboard URL for the App.

            Examples:
                ```python
                app = modal.App.lookup("my-app")
                print(app.get_dashboard_url())
                ```
            """
            ...

        async def aio(self, /) -> str:
            """Get the dashboard URL for the App.

            Returns:
                The dashboard URL for the App.

            Examples:
                ```python
                app = modal.App.lookup("my-app")
                print(app.get_dashboard_url())
                ```
            """
            ...

    get_dashboard_url: __get_dashboard_url_spec

    def set_description(self, description: str):
        """mdmd:hidden
        Set the description of the App before it starts running.

        Note: we don't recommend using the method and may deprecate it in the future.
        """
        ...

    def _validate_blueprint_value(self, key: str, value: typing.Any): ...
    @property
    def image(self) -> modal.image.Image:
        """mdmd:hidden
        Retrieve the Image that will be used as the default for any Functions registered to the App.

        Note: This property is only relevant in the build phase and won't be populated on a deployed
        App that is retrieved via `modal.App.lookup`. It is likely to be deprecated in the future.
        """
        ...

    @image.setter
    def image(self, value):
        """mdmd:hidden"""
        ...

    def _uncreate_all_objects(self): ...

    class ___set_local_app_spec(typing_extensions.Protocol):
        def __call__(
            self, /, client: modal.client.Client, running_app: modal.running_app.RunningApp
        ) -> synchronicity.combined_types.AsyncAndBlockingContextManager[None]: ...
        def aio(
            self, /, client: modal.client.Client, running_app: modal.running_app.RunningApp
        ) -> typing.AsyncContextManager[None]: ...

    _set_local_app: ___set_local_app_spec

    class __run_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
            detach: bool = False,
            interactive: bool = False,
            environment_name: typing.Optional[str] = None,
        ) -> synchronicity.combined_types.AsyncAndBlockingContextManager[App]:
            """Context manager that runs an ephemeral app on Modal.

            Use this as the main entry point for your Modal application. All calls
            to Modal Functions should be made within the scope of this context
            manager, and they will correspond to the current App.

            Note that you should not invoke this in global scope of a file where you have
            Modal Functions or Classes defined, since that would run the block when the Function
            or Cls is imported in your containers as well. If you want to run it as your entrypoint,
            consider protecting it with ``if __name__ == "__main__"``.

            Args:
                client: Modal client to use for the run session.
                detach: Whether to detach after starting the app.
                interactive: Whether to run in interactive mode.
                environment_name: Optional environment name; defaults to the configured environment.

            Returns:
                Async context manager yielding this `App` while it is running.

            Examples:
                ```python notest
                with app.run():
                    some_modal_function.remote()
                ```

                To enable output printing (i.e., to see App logs), use `modal.enable_output()`:

                ```python notest
                with modal.enable_output():
                    with app.run():
                        some_modal_function.remote()
                ```

                Note that you should not invoke this in global scope of a file where you have Modal
                Functions or Classes defined, since that would run the block when the Function or Cls
                is imported in your containers as well. If you want to run it as your entrypoint,
                consider protecting it:

                ```python
                if __name__ == "__main__":
                    with app.run():
                        some_modal_function.remote()
                ```

                You can then run your script with:

                ```shell
                python app_module.py
                ```
            """
            ...

        def aio(
            self,
            /,
            *,
            name: typing.Optional[str] = None,
            client: typing.Optional[modal.client.Client] = None,
            detach: bool = False,
            interactive: bool = False,
            environment_name: typing.Optional[str] = None,
        ) -> typing.AsyncContextManager[App]:
            """Context manager that runs an ephemeral app on Modal.

            Use this as the main entry point for your Modal application. All calls
            to Modal Functions should be made within the scope of this context
            manager, and they will correspond to the current App.

            Note that you should not invoke this in global scope of a file where you have
            Modal Functions or Classes defined, since that would run the block when the Function
            or Cls is imported in your containers as well. If you want to run it as your entrypoint,
            consider protecting it with ``if __name__ == "__main__"``.

            Args:
                client: Modal client to use for the run session.
                detach: Whether to detach after starting the app.
                interactive: Whether to run in interactive mode.
                environment_name: Optional environment name; defaults to the configured environment.

            Returns:
                Async context manager yielding this `App` while it is running.

            Examples:
                ```python notest
                with app.run():
                    some_modal_function.remote()
                ```

                To enable output printing (i.e., to see App logs), use `modal.enable_output()`:

                ```python notest
                with modal.enable_output():
                    with app.run():
                        some_modal_function.remote()
                ```

                Note that you should not invoke this in global scope of a file where you have Modal
                Functions or Classes defined, since that would run the block when the Function or Cls
                is imported in your containers as well. If you want to run it as your entrypoint,
                consider protecting it:

                ```python
                if __name__ == "__main__":
                    with app.run():
                        some_modal_function.remote()
                ```

                You can then run your script with:

                ```shell
                python app_module.py
                ```
            """
            ...

    run: __run_spec

    class __deploy_spec(typing_extensions.Protocol[SUPERSELF]):
        def __call__(
            self,
            /,
            *,
            name: typing.Optional[str] = None,
            environment_name: typing.Optional[str] = None,
            tag: str = "",
            client: typing.Optional[modal.client.Client] = None,
            strategy: str = "rolling",
        ) -> SUPERSELF:
            """Deploy the App so that it is available persistently.

            Deployed Apps will be available for lookup or web-based invocations until they are stopped.
            Unlike with `App.run`, this method will return as soon as the deployment completes.

            This method is a programmatic alternative to the `modal deploy` CLI command.

            Unlike with `App.run`, Function logs will not stream back to the local client after the
            App is deployed.

            Note that you should not invoke this method in global scope, as that would redeploy
            the App every time the file is imported. If you want to write a programmatic deployment
            script, protect this call so that it only runs when the file is executed directly.

            Args:
                name: Name for the deployment, overriding any set on the App.
                environment_name: Environment to deploy the App in.
                tag: Optional metadata that is specific to this deployment.
                client: Alternate client to use for communication with the server.
                strategy:
                    Deployment strategy. ``rolling`` (default) shifts traffic gradually to new containers while old ones
                    drain. ``recreate`` terminates all running containers as part of the deployment before new work starts.

            Returns:
                This app instance after deployment completes.

            Examples:
                ```python notest
                app = App("my-app")
                app.deploy()
                ```

                To enable output printing (i.e., to see build logs), use `modal.enable_output()`:

                ```python notest
                app = App("my-app")
                with modal.enable_output():
                    app.deploy()
                ```

                Unlike with `App.run`, Function logs will not stream back to the local client after the App is deployed.

                Note that you should not invoke this method in global scope, as that would redeploy the App every time
                the file is imported. If you want to write a programmatic deployment script, protect this call so that it
                only runs when the file is executed directly. You can then run your script with:

                ```python notest
                if __name__ == "__main__":
                    with modal.enable_output():
                        app.deploy()
                ```

                Then you can deploy your app with:

                ```shell
                python app_module.py
                ```
            """
            ...

        async def aio(
            self,
            /,
            *,
            name: typing.Optional[str] = None,
            environment_name: typing.Optional[str] = None,
            tag: str = "",
            client: typing.Optional[modal.client.Client] = None,
            strategy: str = "rolling",
        ) -> SUPERSELF:
            """Deploy the App so that it is available persistently.

            Deployed Apps will be available for lookup or web-based invocations until they are stopped.
            Unlike with `App.run`, this method will return as soon as the deployment completes.

            This method is a programmatic alternative to the `modal deploy` CLI command.

            Unlike with `App.run`, Function logs will not stream back to the local client after the
            App is deployed.

            Note that you should not invoke this method in global scope, as that would redeploy
            the App every time the file is imported. If you want to write a programmatic deployment
            script, protect this call so that it only runs when the file is executed directly.

            Args:
                name: Name for the deployment, overriding any set on the App.
                environment_name: Environment to deploy the App in.
                tag: Optional metadata that is specific to this deployment.
                client: Alternate client to use for communication with the server.
                strategy:
                    Deployment strategy. ``rolling`` (default) shifts traffic gradually to new containers while old ones
                    drain. ``recreate`` terminates all running containers as part of the deployment before new work starts.

            Returns:
                This app instance after deployment completes.

            Examples:
                ```python notest
                app = App("my-app")
                app.deploy()
                ```

                To enable output printing (i.e., to see build logs), use `modal.enable_output()`:

                ```python notest
                app = App("my-app")
                with modal.enable_output():
                    app.deploy()
                ```

                Unlike with `App.run`, Function logs will not stream back to the local client after the App is deployed.

                Note that you should not invoke this method in global scope, as that would redeploy the App every time
                the file is imported. If you want to write a programmatic deployment script, protect this call so that it
                only runs when the file is executed directly. You can then run your script with:

                ```python notest
                if __name__ == "__main__":
                    with modal.enable_output():
                        app.deploy()
                ```

                Then you can deploy your app with:

                ```shell
                python app_module.py
                ```
            """
            ...

    deploy: __deploy_spec[typing_extensions.Self]

    def _get_default_image(self): ...
    def _get_watch_mounts(self): ...
    def _add_function(self, function: modal.functions.Function, is_web_endpoint: bool): ...
    def _add_class(self, tag: str, cls: modal.cls.Cls): ...
    def _init_container(self, client: modal.client.Client, running_app: modal.running_app.RunningApp): ...
    @property
    def registered_functions(self) -> dict[str, modal.functions.Function]:
        """mdmd:hidden
        All modal.Function objects registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future in favor of a different
        approach for retrieving the layout of a deployed App.
        """
        ...

    @property
    def registered_classes(self) -> dict[str, modal.cls.Cls]:
        """mdmd:hidden
        All modal.Cls objects registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future in favor of a different
        approach for retrieving the layout of a deployed App.
        """
        ...

    @property
    def registered_entrypoints(self) -> dict[str, LocalEntrypoint]:
        """mdmd:hidden
        All local CLI entrypoints registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future.
        """
        ...

    @property
    def registered_web_endpoints(self) -> list[str]:
        """mdmd:hidden
        Names of Web Functions registered on the app.

        Note: this property is populated only during the build phase, and it is not
        expected to work when a deployed App has been retrieved via `modal.App.lookup`.
        This method is likely to be deprecated in the future in favor of a different
        approach for retrieving the layout of a deployed App.
        """
        ...

    def local_entrypoint(
        self, _warn_parentheses_missing: typing.Any = None, *, name: typing.Optional[str] = None
    ) -> collections.abc.Callable[[collections.abc.Callable[..., typing.Any]], LocalEntrypoint]:
        """Decorate a function to be used as a CLI entrypoint for a Modal App.

        These functions can be used to define code that runs locally to set up the app,
        and act as an entrypoint to start Modal functions from. Note that regular
        Modal functions can also be used as CLI entrypoints, but unlike `local_entrypoint`,
        those functions are executed remotely directly.

        Note that an explicit [`app.run()`](https://modal.com/docs/sdk/py/latest/App#run) is not needed, as an
        [app](https://modal.com/docs/guide/apps) is automatically created for you.

        Args:
            name: Optional name for the entrypoint; defaults to the function's qualified name.

        Returns:
            A decorator that registers the wrapped callable as a local CLI entrypoint.

        Examples:
            ```python
            @app.local_entrypoint()
            def main():
                some_modal_function.remote()
            ```

            You can call the function using `modal run` directly from the CLI:

            ```shell
            modal run app_module.py
            ```

            Note that an explicit `app.run()` is not needed, as an app is automatically created for you.

            **Multiple entrypoints**

            If you have multiple `local_entrypoint` functions, qualify the name:

            ```shell
            modal run app_module.py::app.some_other_function
            ```

            **Parsing arguments**

            If your entrypoint function take arguments with primitive types, `modal run` automatically
            parses them as CLI options. For example, the following function can be called with
            `modal run app_module.py --foo 1 --bar "hello"`:

            ```python
            @app.local_entrypoint()
            def main(foo: int, bar: str):
                some_modal_function.call(foo, bar)
            ```

            Currently, `str`, `int`, `float`, `bool`, and `datetime.datetime` are supported.
            Use `modal run app_module.py --help` for more information on usage.
        """
        ...

    def function(
        self,
        _warn_parentheses_missing=None,
        *,
        image: typing.Optional[modal.image.Image] = None,
        schedule: typing.Optional[modal.schedule.Schedule] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
        gpu: typing.Union[str, list[str], None] = None,
        serialized: bool = False,
        network_file_systems: dict[
            typing.Union[str, pathlib.PurePosixPath], modal.network_file_system.NetworkFileSystem
        ] = {},
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
        ] = {},
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        ephemeral_disk: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        proxy: typing.Optional[modal.proxy.Proxy] = None,
        retries: typing.Union[int, modal.retries.Retries, None] = None,
        timeout: int = 300,
        startup_timeout: typing.Optional[int] = None,
        name: typing.Optional[str] = None,
        is_generator: typing.Optional[bool] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        routing_region: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        enable_memory_snapshot: bool = False,
        block_network: bool = False,
        restrict_modal_access: bool = False,
        single_use_containers: bool = False,
        i6pn: typing.Optional[bool] = None,
        include_source: typing.Optional[bool] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        _experimental_restrict_output: bool = False,
        max_inputs: typing.Optional[int] = None,
    ) -> _FunctionDecoratorType:
        """Decorator to register a new Modal Function with this App.

        Args:
            image: The image to run as the container for the function.
            schedule: An optional Modal Schedule for the function.
            env: Environment variables to set in the container.
            secrets: Secrets to inject into the container as environment variables.
            gpu: GPU request; either a single GPU type or a list of types.
            serialized: Whether to send the function over using cloudpickle.
            network_file_systems: Mountpoints for Modal NetworkFileSystems.
            volumes: Mount points for Modal Volumes & CloudBucketMounts.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container from
                exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            ephemeral_disk: Specify, in MiB, the ephemeral disk size for the Function.
            min_containers: Minimum number of containers to keep warm, even when Function is idle.
            max_containers: Limit on the number of containers that can be concurrently running.
            buffer_containers: Number of additional idle containers to maintain under active load.
            scaledown_window: Max time (in seconds) a container can remain idle while scaling down.
            proxy: Reference to a Modal Proxy to use in front of this function.
            retries: Number of times to retry each input in case of failure.
            timeout: Maximum execution time for inputs and startup time in seconds.
            startup_timeout: Maximum startup time in seconds with higher precedence than `timeout`.
            name: Sets the Modal name of the function within the app.
            is_generator:
                Set this to True if it's a non-generator function returning a sync or async generator object.
            cloud: Cloud provider to run the function on. Possible values are aws, gcp, oci, auto.
            region: Region or regions to run the function on.
            routing_region: Region to route inputs to the function through.
            nonpreemptible: Whether to run the function on a nonpreemptible instance.
            enable_memory_snapshot: Enable memory checkpointing for faster cold starts.
            block_network: Whether to block network access.
            restrict_modal_access: Whether to allow this function access to other Modal resources.
            single_use_containers: When True, containers will shut down after handling a single input.
            i6pn: Whether to enable IPv6 container networking within the region.
            include_source:
                Whether the file or directory containing the Function's source should automatically be included in the
                container. When unset, falls back to the App-level configuration, or is otherwise True by default.
            experimental_options: Experimental options for the function.
            _experimental_restrict_output: Experimental; do not use pickle for return values.
            max_inputs: Deprecated; replaced with `single_use_containers`.

        Returns:
            A decorator that registers the wrapped callable or partial as a Modal `Function`.
        """
        ...

    @typing_extensions.dataclass_transform(
        field_specifiers=(modal.cls.parameter,),
        kw_only_default=True,
    )
    def cls(
        self,
        _warn_parentheses_missing=None,
        *,
        image: typing.Optional[modal.image.Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
        gpu: typing.Union[str, list[str], None] = None,
        serialized: bool = False,
        network_file_systems: dict[
            typing.Union[str, pathlib.PurePosixPath], modal.network_file_system.NetworkFileSystem
        ] = {},
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
        ] = {},
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        ephemeral_disk: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        proxy: typing.Optional[modal.proxy.Proxy] = None,
        retries: typing.Union[int, modal.retries.Retries, None] = None,
        timeout: int = 300,
        startup_timeout: typing.Optional[int] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        routing_region: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        enable_memory_snapshot: bool = False,
        block_network: bool = False,
        restrict_modal_access: bool = False,
        single_use_containers: bool = False,
        i6pn: typing.Optional[bool] = None,
        include_source: typing.Optional[bool] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
        _experimental_restrict_output: bool = False,
        max_inputs: typing.Optional[int] = None,
    ) -> collections.abc.Callable[[typing.Union[CLS_T, modal.partial_function.PartialFunction]], CLS_T]:
        """Decorator to register a new Modal [Cls](https://modal.com/docs/sdk/py/latest/Cls) with this App.

        Args:
            image: The image to run as the container for the class service.
            env: Environment variables to set in the container.
            secrets: Secrets to inject into the container as environment variables.
            gpu: GPU request; either a single GPU type or a list of types.
            serialized: Whether to send the class over using cloudpickle.
            network_file_systems: Mountpoints for Modal NetworkFileSystems.
            volumes: Mount points for Modal Volumes & CloudBucketMounts.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container from
                exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            ephemeral_disk: Specify, in MiB, the ephemeral disk size for the Function.
            min_containers: Minimum number of containers to keep warm, even when Function is idle.
            max_containers: Limit on the number of containers that can be concurrently running.
            buffer_containers: Number of additional idle containers to maintain under active load.
            scaledown_window: Max time (in seconds) a container can remain idle while scaling down.
            proxy: Reference to a Modal Proxy to use in front of this function.
            retries: Number of times to retry each input in case of failure.
            timeout: Maximum execution time for inputs and startup time in seconds.
            startup_timeout: Maximum startup time in seconds with higher precedence than `timeout`.
            cloud: Cloud provider to run the function on. Possible values are aws, gcp, oci, auto.
            region: Region or regions to run the function on.
            routing_region: Region to route inputs to the function through.
            nonpreemptible: Whether to run the function on a non-preemptible instance.
            enable_memory_snapshot: Enable memory checkpointing for faster cold starts.
            block_network: Whether to block network access.
            restrict_modal_access: Whether to allow this class access to other Modal resources.
            single_use_containers: When True, containers will shut down after handling a single input.
            i6pn: Whether to enable IPv6 container networking within the region.
            include_source: When ``False``, don't automatically add the App source to the container.
            experimental_options: Experimental options for the class service.
            _experimental_restrict_output: Experimental; do not use pickle for return values.
            max_inputs: Deprecated; replaced with `single_use_containers`.

        Returns:
            A decorator that registers the wrapped class or partial as a Modal `Cls`.
        """
        ...

    def server(
        self,
        _warn_parentheses_missing=None,
        *,
        image: typing.Optional[modal.image.Image] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
        gpu: typing.Union[str, list[str], None] = None,
        serialized: bool = False,
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
        ] = {},
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        ephemeral_disk: typing.Optional[int] = None,
        target_concurrency: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaleup_window: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        startup_timeout: int = 30,
        name: typing.Optional[str] = None,
        port: int = 8000,
        unauthenticated: bool = False,
        h2_enabled: bool = False,
        exit_grace_period: int = 0,
        routing_region: str = "us-east",
        compute_region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cloud: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        proxy: typing.Optional[modal.proxy.Proxy] = None,
        i6pn: typing.Optional[bool] = None,
        enable_memory_snapshot: bool = False,
        include_source: typing.Optional[bool] = None,
        experimental_options: typing.Optional[dict[str, typing.Any]] = None,
    ) -> collections.abc.Callable[[typing.Union[CLS_T, modal.partial_function.PartialFunction]], modal.server.Server]:
        """Decorator to register a new Modal Server with this App.

        Servers run HTTP servers that are started in a `@modal.enter()` method.
        Unlike `@app.cls()`, servers only expose HTTP endpoints and do not
        support `.remote()` method calls.

        See the [guide](https://modal.com/docs/guide/servers) for more information.

        Args:
            image: The image to run as the container for the server.
            env: Environment variables to set in the container.
            secrets: Secrets to inject into the container as environment variables.
            gpu: GPU request; either a single GPU type or a list of types.
            serialized: Whether to send the server class over using cloudpickle.
            volumes: Mount points for Modal Volumes & CloudBucketMounts.
            cpu:
                Specify, in fractional CPU cores, how many CPU cores to request. Or, pass (request, limit) to
                additionally specify a hard limit in fractional CPU cores. CPU throttling will prevent a container
                from exceeding its specified limit.
            memory:
                Specify, in MiB, a memory request which is the minimum memory required. Or, pass (request, limit) to
                additionally specify a hard limit in MiB.
            ephemeral_disk: Specify, in MiB, the ephemeral disk size for the server.
            target_concurrency: Target concurrency for the server; 0 disables autoscaling.
            min_containers: Minimum number of containers to keep running regardless of demand.
            max_containers: Limit on the number of containers that can be concurrently running.
            buffer_containers: Extra containers to scale up beyond current demand.
            scaleup_window: Seconds of sustained demand required before scaling up new containers.
            scaledown_window: Maximum duration (in seconds) idle containers wait before scaling down.
            startup_timeout: Maximum container startup time in seconds.
            name: Sets the Modal name of the function within the app, defaults to class name.
            port: Port the HTTP server listens on.
            unauthenticated: Whether the endpoint requires proxy authentication; required by default.
            h2_enabled: Enable HTTP/2.
            exit_grace_period: Grace period for in-flight requests on shutdown.
            routing_region: Region to route Server requests through.
            compute_region: Region(s) where containers can be scheduled.
            cloud: Cloud provider (aws, gcp, oci, auto).
            nonpreemptible: Whether to use non-preemptible instances.
            proxy: Modal Proxy to use in front of this server.
            i6pn: Enable IPv6 container networking.
            enable_memory_snapshot: Enable memory checkpointing.
            include_source: Whether to add source to container.
            experimental_options: Experimental options.

        Examples:
            ```python
            @app.server(port=8000, routing_region="us-east")
            class MyServer:
                @modal.enter()
                def start(self):
                    self.proc = subprocess.Popen(["python3", "-m", "http.server", "8000"])

                @modal.exit()
                def stop(self):
                    self.proc.terminate()
            ```
        """
        ...

    def include(self, /, other_app: App, inherit_tags: bool = True) -> typing_extensions.Self:
        """Include another App's objects in this one.

        Useful for splitting up Modal Apps across different self-contained files.

        When `inherit_tags=True` any tags set on the other App will be inherited by this App
        (with this App's tags taking precedence in the case of conflicts).

        Args:
            other_app: App whose registered functions and classes are merged into this app.
            inherit_tags: If True, merge tags from `other_app` into this app (this app wins on conflicts).

        Returns:
            This app instance for chaining.

        Examples:
            ```python
            app_a = modal.App("a")
            @app_a.function()
            def foo():
                ...

            app_b = modal.App("b")
            @app_b.function()
            def bar():
                ...

            app_a.include(app_b)

            @app_a.local_entrypoint()
            def main():
                # use function declared on the included app
                bar.remote()
            ```
        """
        ...

    class __set_tags_spec(typing_extensions.Protocol):
        def __call__(
            self, /, tags: collections.abc.Mapping[str, str], *, client: typing.Optional[modal.client.Client] = None
        ) -> None:
            """Attach key-value metadata to the App.

            Tag metadata can be used to add organization-specific context to the App and can be
            included in billing reports and other informational APIs. Tags can also be set in
            the App constructor.

            Any tags set on the App before calling this method will be removed if they are not
            included in the argument (i.e., this method does not have `.update()` semantics).

            Args:
                tags: Complete tag set to store on the app (replaces previous tags).
                client: Modal client to use for the RPC.
            """
            ...

        async def aio(
            self, /, tags: collections.abc.Mapping[str, str], *, client: typing.Optional[modal.client.Client] = None
        ) -> None:
            """Attach key-value metadata to the App.

            Tag metadata can be used to add organization-specific context to the App and can be
            included in billing reports and other informational APIs. Tags can also be set in
            the App constructor.

            Any tags set on the App before calling this method will be removed if they are not
            included in the argument (i.e., this method does not have `.update()` semantics).

            Args:
                tags: Complete tag set to store on the app (replaces previous tags).
                client: Modal client to use for the RPC.
            """
            ...

    set_tags: __set_tags_spec

    class __get_tags_spec(typing_extensions.Protocol):
        def __call__(self, /, *, client: typing.Optional[modal.client.Client] = None) -> dict[str, str]:
            """Get the tags that are currently attached to the App.

            Args:
                client: Modal client to use for the RPC.

            Returns:
                Tags as a map from key to value.
            """
            ...

        async def aio(self, /, *, client: typing.Optional[modal.client.Client] = None) -> dict[str, str]:
            """Get the tags that are currently attached to the App.

            Args:
                client: Modal client to use for the RPC.

            Returns:
                Tags as a map from key to value.
            """
            ...

    get_tags: __get_tags_spec

    class ___logs_spec(typing_extensions.Protocol):
        def __call__(self, /, client: typing.Optional[modal.client.Client] = None) -> typing.Generator[str, None, None]:
            """Stream logs from the app.

            This method is considered private and its interface may change - use at your own risk!
            """
            ...

        def aio(
            self, /, client: typing.Optional[modal.client.Client] = None
        ) -> collections.abc.AsyncGenerator[str, None]:
            """Stream logs from the app.

            This method is considered private and its interface may change - use at your own risk!
            """
            ...

    _logs: ___logs_spec

    @classmethod
    def _get_container_app(cls) -> typing.Optional[App]:
        """Returns the `App` running inside a container.

        This will return `None` outside of a Modal container.
        """
        ...

    @classmethod
    def _reset_container_app(cls):
        """Only used for tests."""
        ...

_default_image: modal._image._Image
