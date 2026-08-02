import modal._logs_manager
import modal._supports_logs
import modal.app
import modal.client
import modal.functions
import modal.partial_function
import typing
import typing_extensions

class Server:
    """Server runs an HTTP server started in an `@modal.enter` method.

    See the [guide](https://modal.com/docs/guide/servers) for more information.

    Generally, you will not construct a Server directly.
    Instead, use the [`@app.server()`](https://modal.com/docs/sdk/py/latest/App#server) decorator.

    ```python notest
    @app.server(port=8080, routing_region="us-east")
    class MyServer:
        @modal.enter()
        def start_server(self):
            self.process = subprocess.Popen(["python3", "-m", "http.server", "8080"])
    ```
    """

    _user_cls: typing.Optional[type]
    _service_function: modal.functions.Function
    _app: typing.Optional[modal.app.App]

    def __init__(self, /, *args, **kwargs):
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    def _get_user_cls(self) -> type: ...
    def _get_app(self) -> modal.app.App: ...
    def _get_service_function(self) -> modal.functions.Function: ...
    @property
    def object_id(self) -> str:
        """Modal's internal ID for this Server instance."""
        ...

    class ___get_log_query_data_spec(typing_extensions.Protocol):
        def __call__(self, /) -> modal._supports_logs._LogQueryData: ...
        async def aio(self, /) -> modal._supports_logs._LogQueryData: ...

    _get_log_query_data: ___get_log_query_data_spec

    @property
    def logs(self) -> modal._logs_manager.ServerLogsManager:
        """Access logs for a `Server`.

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
    def _extract_user_cls(wrapped_user_cls: typing.Union[type, modal.partial_function.PartialFunction]) -> type: ...

    class __get_url_spec(typing_extensions.Protocol):
        def __call__(self, /) -> typing.Optional[str]:
            """The URL for making requests to this Server."""
            ...

        async def aio(self, /) -> typing.Optional[str]:
            """The URL for making requests to this Server."""
            ...

    get_url: __get_url_spec

    class __update_autoscaler_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            target_concurrency: typing.Optional[int] = None,
            min_containers: typing.Optional[int] = None,
            max_containers: typing.Optional[int] = None,
            buffer_containers: typing.Optional[int] = None,
            scaleup_window: typing.Optional[int] = None,
            scaledown_window: typing.Optional[int] = None,
        ) -> None:
            """Override the current autoscaler behavior for this Server.

            Unspecified parameters will retain their current value, i.e. either the static value
            from the `@app.server()` decorator, or an override value from a previous call to this method.

            Subsequent deployments of the App containing this Server will reset the autoscaler back to
            its static configuration.

            Args:
                target_concurrency: Target number of concurrent requests per container.
                min_containers: Minimum number of containers to keep running regardless of demand.
                max_containers: Limit on the number of containers that can be concurrently running.
                buffer_containers: Extra containers to scale up beyond current demand.
                scaleup_window: Seconds of sustained demand required before scaling up new containers.
                scaledown_window: Maximum duration (in seconds) idle containers wait before scaling down.

            Examples:
                ```python notest
                server = modal.Server.from_name("my-app", "Server")

                # Always have at least 2 containers running, with an extra buffer of 2 containers
                server.update_autoscaler(min_containers=2, buffer_containers=1)

                # Limit this Server to avoid spinning up more than 5 containers
                server.update_autoscaler(max_containers=5)

                # Require 30 seconds of sustained demand before scaling up
                server.update_autoscaler(scaleup_window=30)

                # Adjust Server autoscaling to target 20 concurrent requests per replica
                server.update_autoscaler(target_concurrency=20)

                # Disable the Server autoscaling by setting target_concurrency to 0
                server.update_autoscaler(target_concurrency=0)
                ```
            """
            ...

        async def aio(
            self,
            /,
            *,
            target_concurrency: typing.Optional[int] = None,
            min_containers: typing.Optional[int] = None,
            max_containers: typing.Optional[int] = None,
            buffer_containers: typing.Optional[int] = None,
            scaleup_window: typing.Optional[int] = None,
            scaledown_window: typing.Optional[int] = None,
        ) -> None:
            """Override the current autoscaler behavior for this Server.

            Unspecified parameters will retain their current value, i.e. either the static value
            from the `@app.server()` decorator, or an override value from a previous call to this method.

            Subsequent deployments of the App containing this Server will reset the autoscaler back to
            its static configuration.

            Args:
                target_concurrency: Target number of concurrent requests per container.
                min_containers: Minimum number of containers to keep running regardless of demand.
                max_containers: Limit on the number of containers that can be concurrently running.
                buffer_containers: Extra containers to scale up beyond current demand.
                scaleup_window: Seconds of sustained demand required before scaling up new containers.
                scaledown_window: Maximum duration (in seconds) idle containers wait before scaling down.

            Examples:
                ```python notest
                server = modal.Server.from_name("my-app", "Server")

                # Always have at least 2 containers running, with an extra buffer of 2 containers
                server.update_autoscaler(min_containers=2, buffer_containers=1)

                # Limit this Server to avoid spinning up more than 5 containers
                server.update_autoscaler(max_containers=5)

                # Require 30 seconds of sustained demand before scaling up
                server.update_autoscaler(scaleup_window=30)

                # Adjust Server autoscaling to target 20 concurrent requests per replica
                server.update_autoscaler(target_concurrency=20)

                # Disable the Server autoscaling by setting target_concurrency to 0
                server.update_autoscaler(target_concurrency=0)
                ```
            """
            ...

    update_autoscaler: __update_autoscaler_spec

    class __hydrate_spec(typing_extensions.Protocol):
        def __call__(self, /, client: typing.Optional[modal.client.Client] = None) -> Server:
            """Synchronize the local object with its identity on the Modal server.

            It is rarely necessary to call this method explicitly, as most operations will
            lazily hydrate when needed. The main use case is when you need to access object
            metadata, such as its ID.
            """
            ...

        async def aio(self, /, client: typing.Optional[modal.client.Client] = None) -> Server:
            """Synchronize the local object with its identity on the Modal server.

            It is rarely necessary to call this method explicitly, as most operations will
            lazily hydrate when needed. The main use case is when you need to access object
            metadata, such as its ID.
            """
            ...

    hydrate: __hydrate_spec

    @staticmethod
    def _from_local(
        wrapped_user_cls: typing.Union[type, modal.partial_function.PartialFunction],
        app: modal.app.App,
        service_function: modal.functions.Function,
    ) -> Server:
        """Create a Server from a local class definition."""
        ...

    @classmethod
    def from_name(
        cls: type[Server],
        app_name: str,
        name: str,
        *,
        environment_name: typing.Optional[str] = None,
        client: typing.Optional[modal.client.Client] = None,
    ) -> Server:
        """Reference a Server from a deployed App by its name.

        This is a lazy method that defers hydrating the local
        object with metadata from Modal servers until the first
        time it is actually used.

        Args:
            app_name: Name of the App containing the Server.
            name: Name of the Server within the App.
            environment_name: Name of the Environment where the App is deployed.
            client: Modal client instance for this session.

        ```python notest
        server = modal.Server.from_name("other-app", "Server")
        ```
        """
        ...

    def _is_local(self) -> bool:
        """Returns True if this Server has local source code available."""
        ...

    @staticmethod
    def _validate_wrapped_user_cls_decorators(
        wrapped_user_cls: typing.Union[type, modal.partial_function.PartialFunction], enable_memory_snapshot: bool
    ): ...
    @staticmethod
    def _validate_construction_mechanism(wrapped_user_cls: typing.Union[type, modal.partial_function.PartialFunction]):
        """Validate that the server class doesn't have a custom constructor."""
        ...
