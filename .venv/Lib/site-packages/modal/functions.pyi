import collections.abc
import google.protobuf.message
import modal._function_variants
import modal._functions
import modal._load_context
import modal._logs_manager
import modal._supports_logs
import modal._utils.async_utils
import modal._utils.function_utils
import modal.app
import modal.client
import modal.cloud_bucket_mount
import modal.cls
import modal.image
import modal.mount
import modal.network_file_system
import modal.object
import modal.parallel_map
import modal.proxy
import modal.retries
import modal.schedule
import modal.secret
import modal.types
import modal.volume
import modal_proto.api_pb2
import pathlib
import typing
import typing_extensions

ReturnType_INNER = typing.TypeVar("ReturnType_INNER", covariant=True)

P_INNER = typing.ParamSpec("P_INNER")

class Function(
    typing.Generic[modal._functions.P, modal._functions.ReturnType, modal._functions.OriginalReturnType],
    modal.object.Object,
):
    """Functions are the basic units of serverless execution on Modal.

    Generally, you will not construct a `Function` directly. Instead, use the
    `App.function()` decorator to register your Python functions with your App.
    """

    _info: typing.Optional[modal._utils.function_utils.FunctionInfo]
    _serve_mounts: frozenset[modal.mount.Mount]
    _app: typing.Optional[modal.app.App]
    _obj: typing.Optional[modal.cls.Obj]
    _webhook_config: typing.Optional[modal_proto.api_pb2.WebhookConfig]
    _web_url: typing.Optional[str]
    _function_name: typing.Optional[str]
    _is_method: bool
    _spec: typing.Optional[modal._functions._FunctionSpec]
    _tag: str
    _raw_f: typing.Optional[collections.abc.Callable[..., typing.Any]]
    _build_args: dict
    _is_generator: typing.Optional[bool]
    _use_method_name: str
    _class_parameter_info: typing.Optional[modal_proto.api_pb2.ClassParameterInfo]
    _method_handle_metadata: typing.Optional[dict[str, modal_proto.api_pb2.FunctionHandleMetadata]]
    _metadata: typing.Optional[modal_proto.api_pb2.FunctionHandleMetadata]
    _options: modal._function_variants._FunctionOptions
    _base_function: typing.Optional[Function]
    _app_id: typing.Optional[str]

    def __init__(self, *args, **kwargs):
        """mdmd:hidden"""
        ...

    class ___get_log_query_data_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal._supports_logs._LogQueryData: ...
        async def aio(self, /) -> modal._supports_logs._LogQueryData: ...

    _get_log_query_data: ___get_log_query_data_spec

    @property
    def logs(self) -> modal._logs_manager.FunctionLogsManager:
        """Access logs for a `Function`.

        Use [`fetch()`](#logsfetch)
        to read logs from a UTC time range, [`tail()`](#logstail)
        to read the most recent logs, and [`stream()`](#logsstream)
        to follow new logs as they arrive.

        See also:
            - [`modal app logs`](https://modal.com/docs/cli/latest/app#modal-app-logs):
              CLI access to logs for an App.
        """
        ...

    @staticmethod
    def from_local(
        info: modal._utils.function_utils.FunctionInfo,
        app: typing.Optional[modal.app.App],
        image: modal.image.Image,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
        schedule: typing.Optional[modal.schedule.Schedule] = None,
        is_generator: bool = False,
        gpu: typing.Union[str, list[str], None] = None,
        network_file_systems: dict[
            typing.Union[str, pathlib.PurePosixPath], modal.network_file_system.NetworkFileSystem
        ] = {},
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
        ] = {},
        webhook_config: typing.Optional[modal_proto.api_pb2.WebhookConfig] = None,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        proxy: typing.Optional[modal.proxy.Proxy] = None,
        retries: typing.Union[int, modal.retries.Retries, None] = None,
        timeout: int = 300,
        startup_timeout: typing.Optional[int] = None,
        min_containers: typing.Optional[int] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaleup_window: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        max_concurrent_inputs: typing.Optional[int] = None,
        target_concurrent_inputs: typing.Optional[int] = None,
        batch_max_size: typing.Optional[int] = None,
        batch_wait_ms: typing.Optional[int] = None,
        cloud: typing.Optional[str] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        routing_region: typing.Optional[str] = None,
        nonpreemptible: bool = False,
        is_builder_function: bool = False,
        is_auto_snapshot: bool = False,
        is_server: bool = False,
        enable_memory_snapshot: bool = False,
        block_network: bool = False,
        restrict_modal_access: bool = False,
        i6pn_enabled: bool = False,
        cluster_size: typing.Optional[int] = None,
        rdma: typing.Optional[bool] = None,
        fabric_size: typing.Optional[int] = None,
        single_use_containers: bool = False,
        ephemeral_disk: typing.Optional[int] = None,
        include_source: bool = True,
        experimental_options: typing.Optional[dict[str, str]] = None,
        restrict_output: bool = False,
        http_config: typing.Optional[modal_proto.api_pb2.HTTPConfig] = None,
    ) -> Function:
        """mdmd:hidden

        Note: This is not intended to be public API.
        """
        ...

    class ___update_autoscaler_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            min_containers: typing.Optional[int] = None,
            max_containers: typing.Optional[int] = None,
            buffer_containers: typing.Optional[int] = None,
            scaleup_window: typing.Optional[int] = None,
            scaledown_window: typing.Optional[int] = None,
            target_concurrency: typing.Optional[int] = None,
        ) -> None: ...
        async def aio(
            self,
            /,
            *,
            min_containers: typing.Optional[int] = None,
            max_containers: typing.Optional[int] = None,
            buffer_containers: typing.Optional[int] = None,
            scaleup_window: typing.Optional[int] = None,
            scaledown_window: typing.Optional[int] = None,
            target_concurrency: typing.Optional[int] = None,
        ) -> None: ...

    _update_autoscaler: ___update_autoscaler_spec

    class __update_autoscaler_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            min_containers: typing.Optional[int] = None,
            max_containers: typing.Optional[int] = None,
            buffer_containers: typing.Optional[int] = None,
            scaledown_window: typing.Optional[int] = None,
        ) -> None:
            """Override the current autoscaler behavior for this Function.

            Unspecified parameters will retain their current value, i.e. either the static value
            from the function decorator, or an override value from a previous call to this method.

            Subsequent deployments of the App containing this Function will reset the autoscaler back to
            its static configuration.

            Args:
                min_containers: Minimum number of containers to keep running.
                max_containers: Maximum concurrent containers.
                buffer_containers: Extra containers to keep warm beyond current demand.
                scaledown_window: Maximum duration (in seconds) idle containers wait before scaling down.

            Examples:
                ```python notest
                f = modal.Function.from_name("my-app", "function")

                # Always have at least 2 containers running, with an extra buffer when the Function is active
                f.update_autoscaler(min_containers=2, buffer_containers=1)

                # Limit this Function to avoid spinning up more than 5 containers
                f.update_autoscaler(max_containers=5)

                # Extend the scaledown window to increase the amount of time that idle containers stay alive
                f.update_autoscaler(scaledown_window=300)
                ```
            """
            ...

        async def aio(
            self,
            /,
            *,
            min_containers: typing.Optional[int] = None,
            max_containers: typing.Optional[int] = None,
            buffer_containers: typing.Optional[int] = None,
            scaledown_window: typing.Optional[int] = None,
        ) -> None:
            """Override the current autoscaler behavior for this Function.

            Unspecified parameters will retain their current value, i.e. either the static value
            from the function decorator, or an override value from a previous call to this method.

            Subsequent deployments of the App containing this Function will reset the autoscaler back to
            its static configuration.

            Args:
                min_containers: Minimum number of containers to keep running.
                max_containers: Maximum concurrent containers.
                buffer_containers: Extra containers to keep warm beyond current demand.
                scaledown_window: Maximum duration (in seconds) idle containers wait before scaling down.

            Examples:
                ```python notest
                f = modal.Function.from_name("my-app", "function")

                # Always have at least 2 containers running, with an extra buffer when the Function is active
                f.update_autoscaler(min_containers=2, buffer_containers=1)

                # Limit this Function to avoid spinning up more than 5 containers
                f.update_autoscaler(max_containers=5)

                # Extend the scaledown window to increase the amount of time that idle containers stay alive
                f.update_autoscaler(scaledown_window=300)
                ```
            """
            ...

    update_autoscaler: __update_autoscaler_spec

    @classmethod
    def _from_name(
        cls,
        app_name: str,
        name: str,
        *,
        version: typing.Optional[int] = None,
        load_context_overrides: modal._load_context.LoadContext,
    ): ...
    @classmethod
    def from_name(
        cls: type[Function],
        app_name: str,
        name: str,
        *,
        version: typing.Optional[int] = None,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client.Client] = None,
    ) -> Function:
        """Reference a Function from a deployed App by its name.

        This is a lazy method that defers hydrating the local
        object with metadata from Modal servers until the first
        time it is actually used.

        Args:
            app_name: Name of the deployed App.
            name: Name of the Function within that App. For class methods, use `Cls.from_name` instead.
            environment_name: Environment to look up the App in; defaults to the active environment.
            client: Modal client to use; defaults to `Client.from_env()` when omitted.

        Returns:
            A lazy `Function` handle.

        Examples:
            ```python
            f = modal.Function.from_name("other-app", "function")
            ```

            The `version` parameter allows you to invoke a version-pinned function:

            ```python
            f_v3 = modal.Function.from_name("other-app", "function", version=3)
            ```
        """
        ...

    @property
    def tag(self) -> str:
        """mdmd:hidden"""
        ...

    @property
    def app(self) -> modal.app.App:
        """mdmd:hidden"""
        ...

    @property
    def stub(self) -> modal.app.App:
        """mdmd:hidden"""
        ...

    @property
    def info(self) -> modal._utils.function_utils.FunctionInfo:
        """mdmd:hidden"""
        ...

    @property
    def spec(self) -> modal._functions._FunctionSpec:
        """mdmd:hidden"""
        ...

    def _is_web_endpoint(self) -> bool: ...
    def get_build_def(self) -> str:
        """mdmd:hidden"""
        ...

    def _initialize_from_empty(self): ...
    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    def _get_metadata(self): ...
    def _check_no_web_url(self, fn_name: str): ...

    class __get_web_url_spec(typing_extensions.Protocol):
        def __call__(self, /) -> typing.Optional[str]:
            """URL for addressing a Web Function via HTTP.

            Returns:
                The HTTPS URL for the web endpoint, or `None` if this Function is not a web endpoint.
            """
            ...

        async def aio(self, /) -> typing.Optional[str]:
            """URL for addressing a Web Function via HTTP.

            Returns:
                The HTTPS URL for the web endpoint, or `None` if this Function is not a web endpoint.
            """
            ...

    get_web_url: __get_web_url_spec

    class ___experimental_get_flash_urls_spec(typing_extensions.Protocol):
        def __call__(self, /) -> typing.Optional[list[str]]:
            """URL of the flash service for the function.

            Returns:
                Flash service URLs when configured, or `None`.
            """
            ...

        async def aio(self, /) -> typing.Optional[list[str]]:
            """URL of the flash service for the function.

            Returns:
                Flash service URLs when configured, or `None`.
            """
            ...

    _experimental_get_flash_urls: ___experimental_get_flash_urls_spec

    def _apply_dynamic_config(
        self, new_options: modal._function_variants._FunctionOptions, config_method_name: str
    ) -> Function[modal._functions.P, modal._functions.ReturnType, modal._functions.OriginalReturnType]: ...
    def with_options(
        self,
        *,
        cpu: typing.Union[float, tuple[float, float], None] = None,
        memory: typing.Union[int, tuple[int, int], None] = None,
        gpu: typing.Optional[str] = None,
        env: typing.Optional[dict[str, typing.Optional[str]]] = None,
        secrets: typing.Optional[collections.abc.Collection[modal.secret.Secret]] = None,
        volumes: dict[
            typing.Union[str, pathlib.PurePosixPath],
            typing.Union[modal.volume.Volume, modal.cloud_bucket_mount.CloudBucketMount],
        ] = {},
        retries: typing.Union[int, modal.retries.Retries, None] = None,
        max_containers: typing.Optional[int] = None,
        buffer_containers: typing.Optional[int] = None,
        scaledown_window: typing.Optional[int] = None,
        timeout: typing.Optional[int] = None,
        region: typing.Union[str, collections.abc.Sequence[str], None] = None,
        cloud: typing.Optional[str] = None,
        routing_region: typing.Optional[str] = None,
    ) -> Function[modal._functions.P, modal._functions.ReturnType, modal._functions.OriginalReturnType]:
        """Dynamically override the static Function configuration with invocation-specific values.

        This method returns a new Function instance with the dynamic configuration. Invocations of
        the new Function will run in a distinct container pool and autoscale independently from the
        base Function (and from other dynamic configurations).

        Note that options cannot be "unset" with this method (i.e., if a GPU is configured in the
        `@app.cls()` decorator, passing `gpu=None` here will not create a CPU-only instance).
        Additionally, container arguments like `volumes` and `secrets` will _replace_ the base
        configuration or any previous use of this method rather than extending it.

        **Usage:**

        You can use this method after looking up a deployed Function:

        ```python notest
        fn = modal.Function.from_name("my_app", "fn").with_options(gpu="H100")
        fn.remote()  # will run on a H100 GPU
        ```

        Or by referencing another Function defined in the same App:

        ```python notest
        @app.function()
        def fn():
            ...

        # From a local entrypoint or another Function
        fn.with_options(gpu="H100").remote()  # Uses an H100 GPU
        fn.remote()  # Uses the static configuration with no GPU
        ```
        """
        ...

    def with_concurrency(
        self, *, max_inputs: int, target_inputs: typing.Optional[int] = None
    ) -> Function[modal._functions.P, modal._functions.ReturnType, modal._functions.OriginalReturnType]:
        """Override the static Function configuration with invocation-specific input concurrency.

        Returns a new Function instance that is dynamically configured to behave like a Function using
        the `@modal.concurrent` decorator. This instance will autoscale independently from the base Function.
        """
        ...

    def with_batching(
        self, *, max_batch_size: int, wait_ms: int
    ) -> Function[modal._functions.P, modal._functions.ReturnType, modal._functions.OriginalReturnType]:
        """Override the static Function configuration with invocation-specific dynamic batching.

        Returns a new Function instance that is dynamically configured to behave like a Function using
        the `@modal.batched` decorator. This instance will autoscale independently from the base Function.
        """
        ...

    @property
    def is_generator(self) -> bool:
        """mdmd:hidden"""
        ...

    class ___map_spec(typing_extensions.Protocol):
        def __call__(
            self, /, input_queue: modal.parallel_map.SynchronizedQueue, order_outputs: bool, return_exceptions: bool
        ) -> typing.Generator[typing.Any, None, None]:
            """mdmd:hidden

            Synchronicity-wrapped map implementation. To be safe against invocations of user code in
            the synchronicity thread it doesn't accept an [async]iterator, and instead takes a
              _SynchronizedQueue instance that is fed by higher level functions like .map()

            _SynchronizedQueue is used instead of asyncio.Queue so that the main thread can put
            items in the queue safely.
            """
            ...

        def aio(
            self, /, input_queue: modal.parallel_map.SynchronizedQueue, order_outputs: bool, return_exceptions: bool
        ) -> collections.abc.AsyncGenerator[typing.Any, None]:
            """mdmd:hidden

            Synchronicity-wrapped map implementation. To be safe against invocations of user code in
            the synchronicity thread it doesn't accept an [async]iterator, and instead takes a
              _SynchronizedQueue instance that is fed by higher level functions like .map()

            _SynchronizedQueue is used instead of asyncio.Queue so that the main thread can put
            items in the queue safely.
            """
            ...

    _map: ___map_spec

    class ___spawn_map_spec(typing_extensions.Protocol[ReturnType_INNER]):
        def __call__(self, /, input_queue: modal.parallel_map.SynchronizedQueue) -> FunctionCall[ReturnType_INNER]: ...
        async def aio(self, /, input_queue: modal.parallel_map.SynchronizedQueue) -> FunctionCall[ReturnType_INNER]: ...

    _spawn_map: ___spawn_map_spec[modal._functions.ReturnType]

    class ___call_function_spec(typing_extensions.Protocol[ReturnType_INNER]):
        def __call__(self, /, args, kwargs) -> ReturnType_INNER: ...
        async def aio(self, /, args, kwargs) -> ReturnType_INNER: ...

    _call_function: ___call_function_spec[modal._functions.ReturnType]

    class ___call_function_nowait_spec(typing_extensions.Protocol):
        def __call__(
            self, /, args, kwargs, function_call_invocation_type: int, from_spawn_map: bool = False
        ) -> modal._functions._Invocation: ...
        async def aio(
            self, /, args, kwargs, function_call_invocation_type: int, from_spawn_map: bool = False
        ) -> modal._functions._Invocation: ...

    _call_function_nowait: ___call_function_nowait_spec

    class ___call_generator_spec(typing_extensions.Protocol):
        def __call__(self, /, args, kwargs): ...
        def aio(self, /, args, kwargs): ...

    _call_generator: ___call_generator_spec

    class __remote_spec(typing_extensions.Protocol[P_INNER, ReturnType_INNER]):
        def __call__(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> ReturnType_INNER:
            """Calls the function remotely, executing it with the given arguments and returning the execution's result.

            Args:
                *args: Positional arguments forwarded to the deployed function.
                **kwargs: Keyword arguments forwarded to the deployed function.

            Returns:
                The value returned by the remote function.
            """
            ...

        async def aio(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> ReturnType_INNER:
            """Calls the function remotely, executing it with the given arguments and returning the execution's result.

            Args:
                *args: Positional arguments forwarded to the deployed function.
                **kwargs: Keyword arguments forwarded to the deployed function.

            Returns:
                The value returned by the remote function.
            """
            ...

    remote: __remote_spec[modal._functions.P, modal._functions.ReturnType]

    class __remote_gen_spec(typing_extensions.Protocol):
        def __call__(self, /, *args, **kwargs) -> typing.Generator[typing.Any, None, None]:
            """Calls the generator remotely, executing it with the given arguments.

            Args:
                *args: Positional arguments forwarded to the deployed generator function.
                **kwargs: Keyword arguments forwarded to the deployed generator function.

            Yields:
                Values produced by the remote generator.
            """
            ...

        def aio(self, /, *args, **kwargs) -> collections.abc.AsyncGenerator[typing.Any, None]:
            """Calls the generator remotely, executing it with the given arguments.

            Args:
                *args: Positional arguments forwarded to the deployed generator function.
                **kwargs: Keyword arguments forwarded to the deployed generator function.

            Yields:
                Values produced by the remote generator.
            """
            ...

    remote_gen: __remote_gen_spec

    def _is_local(self): ...
    def _get_info(self) -> modal._utils.function_utils.FunctionInfo: ...
    def _get_obj(self) -> typing.Optional[modal.cls.Obj]: ...
    def local(
        self, *args: modal._functions.P.args, **kwargs: modal._functions.P.kwargs
    ) -> modal._functions.OriginalReturnType:
        """Calls the function locally, executing it with the given arguments and returning the execution's result.

        The function will execute in the same environment as the caller, just like calling the underlying function
        directly in Python. In particular, only secrets available in the caller environment will be available
        through environment variables.

        Args:
            *args: Positional arguments passed to the underlying Python callable.
            **kwargs: Keyword arguments passed to the underlying Python callable.

        Returns:
            The return value of the local call (or a coroutine for async functions).
        """
        ...

    class ___experimental_spawn_spec(typing_extensions.Protocol[P_INNER, ReturnType_INNER]):
        def __call__(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> FunctionCall[ReturnType_INNER]:
            """[Experimental] Calls the function with the given arguments, without waiting for the results.

            This experimental version of the spawn method allows up to 1 million inputs to be spawned.

            Conceptually similar to `multiprocessing.pool.apply_async`, or a Future/Promise in other contexts.

            Args:
                *args: Positional arguments forwarded to the remote function.
                **kwargs: Keyword arguments forwarded to the remote function.

            Returns:
                A `modal.FunctionCall` handle; poll or await results with `.get(timeout=...)`.
            """
            ...

        async def aio(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> FunctionCall[ReturnType_INNER]:
            """[Experimental] Calls the function with the given arguments, without waiting for the results.

            This experimental version of the spawn method allows up to 1 million inputs to be spawned.

            Conceptually similar to `multiprocessing.pool.apply_async`, or a Future/Promise in other contexts.

            Args:
                *args: Positional arguments forwarded to the remote function.
                **kwargs: Keyword arguments forwarded to the remote function.

            Returns:
                A `modal.FunctionCall` handle; poll or await results with `.get(timeout=...)`.
            """
            ...

    _experimental_spawn: ___experimental_spawn_spec[modal._functions.P, modal._functions.ReturnType]

    class ___spawn_map_inner_spec(typing_extensions.Protocol[P_INNER]):
        def __call__(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> None: ...
        async def aio(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> None: ...

    _spawn_map_inner: ___spawn_map_inner_spec[modal._functions.P]

    class __spawn_spec(typing_extensions.Protocol[P_INNER, ReturnType_INNER]):
        def __call__(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> FunctionCall[ReturnType_INNER]:
            """Calls the function with the given arguments, without waiting for the results.

            Conceptually similar to `multiprocessing.pool.apply_async`, or a Future/Promise in other contexts.

            Args:
                *args: Positional arguments forwarded to the remote function.
                **kwargs: Keyword arguments forwarded to the remote function.

            Returns:
                A [`modal.FunctionCall`](https://modal.com/docs/sdk/py/latest/FunctionCall) object
                that can later be polled or waited for using
                [`.get(timeout=...)`](https://modal.com/docs/sdk/py/latest/FunctionCall#get).
            """
            ...

        async def aio(self, /, *args: P_INNER.args, **kwargs: P_INNER.kwargs) -> FunctionCall[ReturnType_INNER]:
            """Calls the function with the given arguments, without waiting for the results.

            Conceptually similar to `multiprocessing.pool.apply_async`, or a Future/Promise in other contexts.

            Args:
                *args: Positional arguments forwarded to the remote function.
                **kwargs: Keyword arguments forwarded to the remote function.

            Returns:
                A [`modal.FunctionCall`](https://modal.com/docs/sdk/py/latest/FunctionCall) object
                that can later be polled or waited for using
                [`.get(timeout=...)`](https://modal.com/docs/sdk/py/latest/FunctionCall#get).
            """
            ...

    spawn: __spawn_spec[modal._functions.P, modal._functions.ReturnType]

    def get_raw_f(self) -> collections.abc.Callable[..., typing.Any]:
        """Return the inner Python object wrapped by this Modal Function.

        Returns:
            The original function object registered with Modal.
        """
        ...

    class __get_current_stats_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal.types.FunctionStats:
            """Return a `FunctionStats` object describing the current function's queue and runner counts.

            Returns:
                Snapshot counts for backlog, runners, and running inputs.
            """
            ...

        async def aio(self, /) -> modal.types.FunctionStats:
            """Return a `FunctionStats` object describing the current function's queue and runner counts.

            Returns:
                Snapshot counts for backlog, runners, and running inputs.
            """
            ...

    get_current_stats: __get_current_stats_spec

    class ___get_schema_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal_proto.api_pb2.FunctionSchema:
            """Returns recorded schema for function, internal use only for now"""
            ...

        async def aio(self, /) -> modal_proto.api_pb2.FunctionSchema:
            """Returns recorded schema for function, internal use only for now"""
            ...

    _get_schema: ___get_schema_spec

    class __map_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *input_iterators,
            kwargs={},
            order_outputs: bool = True,
            return_exceptions: bool = False,
            wrap_returned_exceptions: typing.Optional[bool] = None,
        ) -> modal._utils.async_utils.AsyncOrSyncIterable:
            """Parallel map over a set of inputs.

            Pass one iterable per positional argument of the underlying function. Results are yielded as an
            iterable (sync) or async iterator (``map.aio``).

            If applied to an ``@app.function``, ``map()`` returns one result per input and output order matches
            input order by default. Set ``order_outputs=False`` to emit results in completion order.

            ``return_exceptions`` can aggregate failures into the result stream instead of raising.

            Args:
                *input_iterators: One iterator per mapped positional parameter on the function.
                kwargs: Extra keyword arguments forwarded to every invocation.
                order_outputs: If True, preserve input order in outputs; if False, use completion order.
                return_exceptions: If True, failed inputs appear as exceptions in the result stream instead of raising.
                wrap_returned_exceptions: Deprecated; no longer has any effect.

            Examples:
                ```python
                @app.function()
                def my_func(a):
                    return a ** 2


                @app.local_entrypoint()
                def main():
                    assert list(my_func.map([1, 2, 3, 4])) == [1, 4, 9, 16]
                ```

                ```python
                @app.function()
                def my_func(a):
                    if a == 2:
                        raise Exception("ohno")
                    return a ** 2


                @app.local_entrypoint()
                def main():
                    print(list(my_func.map(range(3), return_exceptions=True)))
                ```
            """
            ...

        def aio(
            self,
            /,
            *input_iterators: typing.Union[typing.Iterable[typing.Any], typing.AsyncIterable[typing.Any]],
            kwargs={},
            order_outputs: bool = True,
            return_exceptions: bool = False,
            wrap_returned_exceptions: typing.Optional[bool] = None,
        ) -> typing.AsyncGenerator[typing.Any, None]: ...

    map: __map_spec

    class __starmap_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            input_iterator: typing.Iterable[typing.Sequence[typing.Any]],
            *,
            kwargs={},
            order_outputs: bool = True,
            return_exceptions: bool = False,
            wrap_returned_exceptions: typing.Optional[bool] = None,
        ) -> modal._utils.async_utils.AsyncOrSyncIterable:
            """Like ``map``, but each input item is unpacked into multiple positional arguments.

            Every element of ``input_iterator`` should be a sequence (for example a tuple) with length equal to the
            arity of the function.

            Args:
                input_iterator: Iterable of argument tuples to unpack into each call.
                kwargs: Extra keyword arguments forwarded to every invocation.
                order_outputs: If True, preserve input order in outputs; if False, use completion order.
                return_exceptions: If True, failed inputs appear as exceptions in the result stream instead of raising.
                wrap_returned_exceptions: Deprecated; no longer has any effect.

            Examples:
                ```python
                @app.function()
                def my_func(a, b):
                    return a + b


                @app.local_entrypoint()
                def main():
                    assert list(my_func.starmap([(1, 2), (3, 4)])) == [3, 7]
                ```
            """
            ...

        def aio(
            self,
            /,
            input_iterator: typing.Union[
                typing.Iterable[typing.Sequence[typing.Any]], typing.AsyncIterable[typing.Sequence[typing.Any]]
            ],
            *,
            kwargs={},
            order_outputs: bool = True,
            return_exceptions: bool = False,
            wrap_returned_exceptions: typing.Optional[bool] = None,
        ) -> typing.AsyncIterable[typing.Any]: ...

    starmap: __starmap_spec

    class __for_each_spec(typing_extensions.Protocol):
        def __call__(self, /, *input_iterators, kwargs={}, ignore_exceptions: bool = False):
            """Execute the function for all inputs and wait for completion, discarding return values.

            Like ``.map()`` but you do not need to iterate the result to drive work—Modal processes every input.

            Args:
                *input_iterators: One iterator per mapped positional parameter on the function.
                kwargs: Extra keyword arguments forwarded to every invocation.
                ignore_exceptions: If True, failures are swallowed instead of propagating.
            """
            ...

        async def aio(self, /, *input_iterators, kwargs={}, ignore_exceptions: bool = False) -> None: ...

    for_each: __for_each_spec

    class __spawn_map_spec(typing_extensions.Protocol):
        def __call__(self, /, *input_iterators, kwargs={}) -> None:
            """Spawn parallel execution over a set of inputs, exiting as soon as the inputs are created (without waiting
            for the map to complete).

            Takes one iterator argument per argument in the function being mapped over.

            Programmatic retrieval of results will be supported in a future update.

            Args:
                *input_iterators: One iterator per mapped positional parameter on the function.
                kwargs: Extra keyword arguments forwarded to every invocation.

            Examples:
                ```python
                @app.function()
                def my_func(a):
                    return a ** 2


                @app.local_entrypoint()
                def main():
                    my_func.spawn_map([1, 2, 3, 4])
                ```
            """
            ...

        async def aio(self, /, *input_iterators, kwargs={}) -> None:
            """This runs in an event loop on the main thread. It consumes inputs from the input iterators and creates async
            function calls for each.
            """
            ...

    spawn_map: __spawn_map_spec

    class __experimental_spawn_map_spec(typing_extensions.Protocol):
        def __call__(self, /, *input_iterators, kwargs={}) -> modal._functions._FunctionCall:
            """mdmd:hidden
            Spawn parallel execution over a set of inputs, returning as soon as the inputs are created.

            Unlike `modal.Function.map`, this method does not block on completion of the remote execution but
            returns a `modal.FunctionCall` object that can be used to poll status and retrieve results later.

            Takes one iterator argument per argument in the function being mapped over.

            Examples:
                ```python
                @app.function()
                def my_func(a, b):
                    return a ** b


                @app.local_entrypoint()
                def main():
                    fc = my_func.spawn_map([1, 2], [3, 4])
                ```
            """
            ...

        async def aio(self, /, *input_iterators, kwargs={}) -> modal._functions._FunctionCall: ...

    experimental_spawn_map: __experimental_spawn_map_spec

class FunctionCall(typing.Generic[modal._functions.ReturnType], modal.object.Object):
    """A reference to an executed function call.

    Constructed using `.spawn(...)` on a Modal function with the same
    arguments that a function normally takes. Acts as a reference to
    an ongoing function call that can be passed around and used to
    poll or fetch function results at some later time.

    Conceptually similar to a Future/Promise/AsyncResult in other contexts and languages.
    """

    _is_generator: bool
    _num_inputs: typing.Optional[int]
    _app_id: typing.Optional[str]
    _function_id: typing.Optional[str]

    def __init__(self, *args, **kwargs):
        """mdmd:hidden"""
        ...

    def _invocation(self): ...

    class ___hydrate_from_id_metadata_spec(typing_extensions.Protocol):
        def __call__(self, /) -> None:
            """Hydrate metadata only when needed for FunctionCall fields."""
            ...

        async def aio(self, /) -> None:
            """Hydrate metadata only when needed for FunctionCall fields."""
            ...

    _hydrate_from_id_metadata: ___hydrate_from_id_metadata_spec

    class ___get_log_query_data_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal._supports_logs._LogQueryData: ...
        async def aio(self, /) -> modal._supports_logs._LogQueryData: ...

    _get_log_query_data: ___get_log_query_data_spec

    @property
    def logs(self) -> modal._logs_manager.FunctionCallLogsManager:
        """Access logs for a single `FunctionCall`.

        Use [`fetch()`](#logsfetch)
        to read logs from a UTC time range, [`tail()`](#logstail)
        to read the most recent logs, and [`stream()`](#logsstream)
        to follow new logs as they arrive.

        See also:
            - [`modal app logs`](https://modal.com/docs/cli/latest/app#modal-app-logs):
            CLI access to logs for an App.
        """
        ...

    class __num_inputs_spec(typing_extensions.Protocol):
        def __call__(self, /) -> int:
            """Get the number of inputs in the function call.

            Returns:
                How many inputs this function call includes (e.g. `1` for `.spawn()`, more for `.spawn_map()`).
            """
            ...

        async def aio(self, /) -> int:
            """Get the number of inputs in the function call.

            Returns:
                How many inputs this function call includes (e.g. `1` for `.spawn()`, more for `.spawn_map()`).
            """
            ...

    num_inputs: __num_inputs_spec

    class __get_spec(typing_extensions.Protocol[ReturnType_INNER]):
        def __call__(self, /, timeout: typing.Optional[float] = None, *, index: int = 0) -> ReturnType_INNER:
            """Get the result of the index-th input of the function call.

            `.spawn()` calls have a single output, so only specifying `index=0` is valid.
            A non-zero index is useful when your function has multiple outputs, like via `.spawn_map()`.

            This function waits indefinitely by default. It takes an optional
            `timeout` argument that specifies the maximum number of seconds to wait,
            which can be set to `0` to poll for an output immediately.

            The returned coroutine is not cancellation-safe.

            Args:
                timeout: Maximum seconds to wait for a result, or `None` to wait indefinitely.
                index: Which input's result to retrieve (typically `0` for `.spawn()`).

            Returns:
                The deserialized return value from that input.
            """
            ...

        async def aio(self, /, timeout: typing.Optional[float] = None, *, index: int = 0) -> ReturnType_INNER:
            """Get the result of the index-th input of the function call.

            `.spawn()` calls have a single output, so only specifying `index=0` is valid.
            A non-zero index is useful when your function has multiple outputs, like via `.spawn_map()`.

            This function waits indefinitely by default. It takes an optional
            `timeout` argument that specifies the maximum number of seconds to wait,
            which can be set to `0` to poll for an output immediately.

            The returned coroutine is not cancellation-safe.

            Args:
                timeout: Maximum seconds to wait for a result, or `None` to wait indefinitely.
                index: Which input's result to retrieve (typically `0` for `.spawn()`).

            Returns:
                The deserialized return value from that input.
            """
            ...

    get: __get_spec[modal._functions.ReturnType]

    class __iter_spec(typing_extensions.Protocol[ReturnType_INNER]):
        def __call__(self, /, *, start: int = 0, end: typing.Optional[int] = None) -> typing.Iterator[ReturnType_INNER]:
            """Iterate in-order over the results of the function call.

            Optionally, specify a range [start, end) to iterate over.

            If `end` is not provided, it will iterate over all results.

            Args:
                start: First input index to include (inclusive).
                end: One past the last index to include, or `None` for all remaining inputs.

            Yields:
                Each result value in index order.

            Examples:
                ```python
                @app.function()
                def my_func(a):
                    return a ** 2


                @app.local_entrypoint()
                def main():
                    fc = my_func.spawn_map([1, 2, 3, 4])
                    assert list(fc.iter()) == [1, 4, 9, 16]
                    assert list(fc.iter(start=1, end=3)) == [4, 9]
                ```
            """
            ...

        def aio(
            self, /, *, start: int = 0, end: typing.Optional[int] = None
        ) -> collections.abc.AsyncIterator[ReturnType_INNER]:
            """Iterate in-order over the results of the function call.

            Optionally, specify a range [start, end) to iterate over.

            If `end` is not provided, it will iterate over all results.

            Args:
                start: First input index to include (inclusive).
                end: One past the last index to include, or `None` for all remaining inputs.

            Yields:
                Each result value in index order.

            Examples:
                ```python
                @app.function()
                def my_func(a):
                    return a ** 2


                @app.local_entrypoint()
                def main():
                    fc = my_func.spawn_map([1, 2, 3, 4])
                    assert list(fc.iter()) == [1, 4, 9, 16]
                    assert list(fc.iter(start=1, end=3)) == [4, 9]
                ```
            """
            ...

    iter: __iter_spec[modal._functions.ReturnType]

    class __get_call_graph_spec(typing_extensions.Protocol):
        def __call__(self, /) -> list[modal.types.InputInfo]:
            """Fetch information about the graph of Inputs this FunctionCall is part of.

            Note: the call graph data is not populated in real-time, and its capture is best-effort.
            We do not recommend relying on this method for critical use cases.

            See the [`modal.types`](/docs/sdk/py/latest/types) reference for information
            on the return values.

            Returns:
                A list of `InputInfo` nodes describing the call graph.
            """
            ...

        async def aio(self, /) -> list[modal.types.InputInfo]:
            """Fetch information about the graph of Inputs this FunctionCall is part of.

            Note: the call graph data is not populated in real-time, and its capture is best-effort.
            We do not recommend relying on this method for critical use cases.

            See the [`modal.types`](/docs/sdk/py/latest/types) reference for information
            on the return values.

            Returns:
                A list of `InputInfo` nodes describing the call graph.
            """
            ...

    get_call_graph: __get_call_graph_spec

    class __cancel_spec(typing_extensions.Protocol):
        def __call__(self, /, terminate_containers: bool = False):
            """Cancel the FunctionCall and terminate its inputs without retrying.

            Args:
                terminate_containers: If True, terminate the containers running the cancelled
                    inputs. Any other inputs running concurrently on those containers will be
                    rescheduled.
            """
            ...

        async def aio(self, /, terminate_containers: bool = False):
            """Cancel the FunctionCall and terminate its inputs without retrying.

            Args:
                terminate_containers: If True, terminate the containers running the cancelled
                    inputs. Any other inputs running concurrently on those containers will be
                    rescheduled.
            """
            ...

    cancel: __cancel_spec

    class __from_id_spec(typing_extensions.Protocol):
        def __call__(
            self, /, function_call_id: str, client: typing.Optional[modal.client.Client] = None
        ) -> FunctionCall[typing.Any]:
            """Instantiate a FunctionCall object from an existing ID.

            Note that it's only necessary to re-instantiate the `FunctionCall` with this method
            if you no longer have access to the original object returned from `Function.spawn`.

            Args:
                function_call_id: Object ID of an existing function call (e.g. from `FunctionCall.object_id`).
                client: Modal client to use; defaults to `Client.from_env()` when omitted.

            Returns:
                A `FunctionCall` handle for the given ID.

            Examples:
                ```python notest
                # Spawn a FunctionCall and keep track of its object ID
                fc = my_func.spawn()
                fc_id = fc.object_id

                # Later, use the ID to re-instantiate the FunctionCall object
                fc = FunctionCall.from_id(fc_id)
                result = fc.get()
                ```
            """
            ...

        async def aio(self, /, function_call_id: str, client: typing.Optional[modal.client.Client] = None): ...

    from_id: typing.ClassVar[__from_id_spec]

    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...

    class __gather_spec(typing_extensions.Protocol):
        def __call__(self, /, *function_calls: FunctionCall[modal._functions.T]) -> typing.Sequence[modal._functions.T]:
            """Wait until all Modal FunctionCall objects have results before returning.

            Accepts a variable number of `FunctionCall` objects, as returned by `Function.spawn()`.

            Raises an exception from the first failing function call.

            Args:
                *function_calls: `FunctionCall` instances to wait on (same order as the returned sequence).

            Returns:
                Results in the same order as `function_calls` (like `asyncio.gather`).

            Examples:
                ```python notest
                fc1 = slow_func_1.spawn()
                fc2 = slow_func_2.spawn()

                result_1, result_2 = modal.FunctionCall.gather(fc1, fc2)
                ```

            *Added in v0.73.69*: This method replaces the deprecated `modal.functions.gather` function.
            """
            ...

        async def aio(
            self, /, *function_calls: FunctionCall[modal._functions.T]
        ) -> typing.Sequence[modal._functions.T]:
            """Wait until all Modal FunctionCall objects have results before returning.

            Accepts a variable number of `FunctionCall` objects, as returned by `Function.spawn()`.

            Raises an exception from the first failing function call.

            Args:
                *function_calls: `FunctionCall` instances to wait on (same order as the returned sequence).

            Returns:
                Results in the same order as `function_calls` (like `asyncio.gather`).

            Examples:
                ```python notest
                fc1 = slow_func_1.spawn()
                fc2 = slow_func_2.spawn()

                result_1, result_2 = modal.FunctionCall.gather(fc1, fc2)
                ```

            *Added in v0.73.69*: This method replaces the deprecated `modal.functions.gather` function.
            """
            ...

    gather: typing.ClassVar[__gather_spec]
