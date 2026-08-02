import collections.abc
import datetime
import google.protobuf.message
import modal._environments
import modal.client
import modal.object
import modal.types
import modal_proto.api_pb2
import synchronicity
import typing
import typing_extensions

class EnvironmentManager:
    """Namespace with methods for managing Environment objects."""
    def __init__(self, /, *args, **kwargs):
        """Initialize self.  See help(type(self)) for accurate signature."""
        ...

    class __create_spec(typing_extensions.Protocol):
        def __call__(
            self, /, name: str, *, restricted: bool = False, client: typing.Optional[modal.client.Client] = None
        ) -> None:
            """Create a new Environment.

            **Examples:**

            ```python notest
            modal.Environment.objects.create("my-environment")
            ```
            """
            ...

        async def aio(
            self, /, name: str, *, restricted: bool = False, client: typing.Optional[modal.client.Client] = None
        ) -> None:
            """Create a new Environment.

            **Examples:**

            ```python notest
            modal.Environment.objects.create("my-environment")
            ```
            """
            ...

    create: __create_spec

    class __list_spec(typing_extensions.Protocol):
        def __call__(self, /, *, client: typing.Optional[modal.client.Client] = None) -> list[Environment]:
            """Return a list of hydrated Environment objects.

            **Examples:**

            ```python notest
            environments = modal.Environment.objects.list()
            print([e.name for e in environments])
            ```
            """
            ...

        async def aio(self, /, *, client: typing.Optional[modal.client.Client] = None) -> list[Environment]:
            """Return a list of hydrated Environment objects.

            **Examples:**

            ```python notest
            environments = modal.Environment.objects.list()
            print([e.name for e in environments])
            ```
            """
            ...

    list: __list_spec

    class __delete_spec(typing_extensions.Protocol):
        def __call__(self, /, name: str, *, client: typing.Optional[modal.client.Client] = None) -> None:
            """Delete a named Environment.

            Warning: This is irreversible and will transitively delete all objects in the Environment.

            **Examples:**

            ```python notest
            modal.Environment.objects.delete("my-environment")
            ```
            """
            ...

        async def aio(self, /, name: str, *, client: typing.Optional[modal.client.Client] = None) -> None:
            """Delete a named Environment.

            Warning: This is irreversible and will transitively delete all objects in the Environment.

            **Examples:**

            ```python notest
            modal.Environment.objects.delete("my-environment")
            ```
            """
            ...

    delete: __delete_spec

class EnvironmentRolesManager:
    """mdmd:namespace"""
    def __init__(self, environment: Environment):
        """mdmd:hidden"""
        ...

    class __list_spec(typing_extensions.Protocol):
        def __call__(self, /) -> dict[typing.Literal["users", "service_users"], dict[str, str]]:
            """Enumerate the Environment Role for each user and service user in the workspace.

            **Examples:**

            ```python notest
            roles = modal.Environment.from_name("my-env").roles.list()
            print(roles)
            # {
            #     "users": {"alice": "contributor", "bob": "viewer", "carol": "contributor"},
            #     "service_users": {"alice-bot": "contributor", "ops-bot": "viewer", "ci-bot": "no-access"},
            # }
            ```
            """
            ...

        async def aio(self, /) -> dict[typing.Literal["users", "service_users"], dict[str, str]]:
            """Enumerate the Environment Role for each user and service user in the workspace.

            **Examples:**

            ```python notest
            roles = modal.Environment.from_name("my-env").roles.list()
            print(roles)
            # {
            #     "users": {"alice": "contributor", "bob": "viewer", "carol": "contributor"},
            #     "service_users": {"alice-bot": "contributor", "ops-bot": "viewer", "ci-bot": "no-access"},
            # }
            ```
            """
            ...

    list: __list_spec

    class __update_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            users: typing.Optional[collections.abc.Mapping[str, str]] = None,
            service_users: typing.Optional[collections.abc.Mapping[str, str]] = None,
        ) -> None:
            """Update the Environment Role of users and service users.

            Each role is one of 'contributor', 'viewer', or 'no-access'. Service users can be
            assigned a role on any Environment, while workspace members can only be assigned a
            role on restricted Environments.

            **Examples:**

            ```python notest
            env = modal.Environment.from_name("my-restricted-env")
            env.roles.update(
                users={"alice": "contributor", "bob": "viewer"},
                service_users={"alice-bot": "contributor"},
            )
            ```
            """
            ...

        async def aio(
            self,
            /,
            *,
            users: typing.Optional[collections.abc.Mapping[str, str]] = None,
            service_users: typing.Optional[collections.abc.Mapping[str, str]] = None,
        ) -> None:
            """Update the Environment Role of users and service users.

            Each role is one of 'contributor', 'viewer', or 'no-access'. Service users can be
            assigned a role on any Environment, while workspace members can only be assigned a
            role on restricted Environments.

            **Examples:**

            ```python notest
            env = modal.Environment.from_name("my-restricted-env")
            env.roles.update(
                users={"alice": "contributor", "bob": "viewer"},
                service_users={"alice-bot": "contributor"},
            )
            ```
            """
            ...

    update: __update_spec

    class ___dispatch_role_updates_spec(typing_extensions.Protocol):
        def __call__(self, /, requests: dict[str, modal_proto.api_pb2.EnvironmentRoleSetRequest]) -> None:
            """Send batched EnvironmentRoleSet RPCs and report all errors encountered."""
            ...

        async def aio(self, /, requests: dict[str, modal_proto.api_pb2.EnvironmentRoleSetRequest]) -> None:
            """Send batched EnvironmentRoleSet RPCs and report all errors encountered."""
            ...

    _dispatch_role_updates: ___dispatch_role_updates_spec

class EnvironmentMembersManager:
    """mdmd:hidden"""
    def __init__(self, environment: Environment):
        """mdmd:hidden"""
        ...

    class __list_spec(typing_extensions.Protocol):
        def __call__(self, /) -> dict[typing.Literal["users", "service_users"], dict[str, str]]: ...
        async def aio(self, /) -> dict[typing.Literal["users", "service_users"], dict[str, str]]: ...

    list: __list_spec

    class __update_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            users: typing.Optional[collections.abc.Mapping[str, str]] = None,
            service_users: typing.Optional[collections.abc.Mapping[str, str]] = None,
        ) -> None: ...
        async def aio(
            self,
            /,
            *,
            users: typing.Optional[collections.abc.Mapping[str, str]] = None,
            service_users: typing.Optional[collections.abc.Mapping[str, str]] = None,
        ) -> None: ...

    update: __update_spec

    class __remove_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            users: typing.Optional[collections.abc.Iterable[str]] = None,
            service_users: typing.Optional[collections.abc.Iterable[str]] = None,
        ) -> None:
            """Remove the Environment Role of users and service users, reverting them to the default."""
            ...

        async def aio(
            self,
            /,
            *,
            users: typing.Optional[collections.abc.Iterable[str]] = None,
            service_users: typing.Optional[collections.abc.Iterable[str]] = None,
        ) -> None:
            """Remove the Environment Role of users and service users, reverting them to the default."""
            ...

    remove: __remove_spec

class EnvironmentBillingManager:
    """mdmd:namespace"""
    def __init__(self, environment: Environment):
        """mdmd:ignore"""
        ...

    class __report_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            *,
            start: datetime.datetime,
            end: typing.Optional[datetime.datetime] = None,
            resolution: str = "d",
            tag_names: typing.Optional[list[str]] = None,
        ) -> list[modal.types.BillingReportItem]:
            """Return a cost report for Environment usage, broken down by object and time.

            Args:
                start: Start of the report, inclusive and rounded to the beginning of the interval.
                    Must be in UTC or timezone-naive (interpreted as UTC).
                end: End of the report, exclusive. Must be in UTC or timezone-naive. Partial final
                    intervals will be excluded from the report.
                resolution: Resolution, e.g. "d" for daily or "h" for hourly.
                tag_names: List of tag names; each row will include the tag name and value in use
                    for that object during the relevant time interval. Pass `["*"]` to include all
                    tags in the report.

            Returns:
                A list of `BillingReportItem` dataclasses. Each item reports the cost attributed to
                a specific Modal object during a given time interval. Cost is further broken down by
                the resource type that generated it (e.g. CPU, Memory, specific GPU usage).
                Note that the specific resource types included in the breakdown are subject to change
                as Modal's billing model evolves.

            See also:
                - [`modal environment billing report`](https://modal.com/docs/cli/latest/environment#modal-environment-billing-report):
                  An environment report CLI that has convenience features around relative time range queries
                  and JSON/CSV output.
                - [`Workspace.billing.report()`](https://modal.com/docs/sdk/py/latest/Workspace#billingreport):
                  An analogous report API for the entire Workspace.
            """
            ...

        async def aio(
            self,
            /,
            *,
            start: datetime.datetime,
            end: typing.Optional[datetime.datetime] = None,
            resolution: str = "d",
            tag_names: typing.Optional[list[str]] = None,
        ) -> list[modal.types.BillingReportItem]:
            """Return a cost report for Environment usage, broken down by object and time.

            Args:
                start: Start of the report, inclusive and rounded to the beginning of the interval.
                    Must be in UTC or timezone-naive (interpreted as UTC).
                end: End of the report, exclusive. Must be in UTC or timezone-naive. Partial final
                    intervals will be excluded from the report.
                resolution: Resolution, e.g. "d" for daily or "h" for hourly.
                tag_names: List of tag names; each row will include the tag name and value in use
                    for that object during the relevant time interval. Pass `["*"]` to include all
                    tags in the report.

            Returns:
                A list of `BillingReportItem` dataclasses. Each item reports the cost attributed to
                a specific Modal object during a given time interval. Cost is further broken down by
                the resource type that generated it (e.g. CPU, Memory, specific GPU usage).
                Note that the specific resource types included in the breakdown are subject to change
                as Modal's billing model evolves.

            See also:
                - [`modal environment billing report`](https://modal.com/docs/cli/latest/environment#modal-environment-billing-report):
                  An environment report CLI that has convenience features around relative time range queries
                  and JSON/CSV output.
                - [`Workspace.billing.report()`](https://modal.com/docs/sdk/py/latest/Workspace#billingreport):
                  An analogous report API for the entire Workspace.
            """
            ...

    report: __report_spec

    class __summary_spec(typing_extensions.Protocol):
        def __call__(
            self, /, cycle: typing.Union[str, datetime.datetime, None] = None
        ) -> modal.types.EnvironmentBillingSummary:
            """Return a summary of environment cost over a single billing cycle determined by `cycle`.

            Unlike the analogous `Workspace.billing.summary()`, this API only emits metered cost
            information. This is because billing adjustments due to credits, free storage, etc. are
            applied at the Workspace level, and thus cannot be attributed to individual Environments.

            Args:
                cycle: Start of the summary, inclusive. Must be the first of a month, and must be in UTC
                    or timezone-naive (interpreted as UTC). If provided as a string, it must either be
                    formatted as an ISO 8601 month (YYYY-MM), or must be one of the convenience spellings
                    "this month" or "last month". If not provided, `cycle` defaults to the first of the
                    current month (in which case a summary is generated for the current billing cycle).

            Returns:
                A single `EnvironmentBillingSummary` dataclass containing the following fields:
                - `metered_cost` representing cost before any adjustments, and
                - `metered_cost_breakdown` containing a breakdown of that cost by the Modal resources
                  that generated it. The exact keys of this are subject to change as Modal's billing
                  model evolves.

                All values are reported as `decimal.Decimal`s.

            See also:
                - [`modal environment billing summary`](https://modal.com/docs/cli/latest/billing#modal-environment-billing-summary):
                  An environment summary CLI that has convenience features around relative time range queries.
                - [`Environment.billing.report()`](https://modal.com/docs/sdk/py/latest/Environment#billingreport):
                  An analogous report API that is scoped to a specific Environment.
            """
            ...

        async def aio(
            self, /, cycle: typing.Union[str, datetime.datetime, None] = None
        ) -> modal.types.EnvironmentBillingSummary:
            """Return a summary of environment cost over a single billing cycle determined by `cycle`.

            Unlike the analogous `Workspace.billing.summary()`, this API only emits metered cost
            information. This is because billing adjustments due to credits, free storage, etc. are
            applied at the Workspace level, and thus cannot be attributed to individual Environments.

            Args:
                cycle: Start of the summary, inclusive. Must be the first of a month, and must be in UTC
                    or timezone-naive (interpreted as UTC). If provided as a string, it must either be
                    formatted as an ISO 8601 month (YYYY-MM), or must be one of the convenience spellings
                    "this month" or "last month". If not provided, `cycle` defaults to the first of the
                    current month (in which case a summary is generated for the current billing cycle).

            Returns:
                A single `EnvironmentBillingSummary` dataclass containing the following fields:
                - `metered_cost` representing cost before any adjustments, and
                - `metered_cost_breakdown` containing a breakdown of that cost by the Modal resources
                  that generated it. The exact keys of this are subject to change as Modal's billing
                  model evolves.

                All values are reported as `decimal.Decimal`s.

            See also:
                - [`modal environment billing summary`](https://modal.com/docs/cli/latest/billing#modal-environment-billing-summary):
                  An environment summary CLI that has convenience features around relative time range queries.
                - [`Environment.billing.report()`](https://modal.com/docs/sdk/py/latest/Environment#billingreport):
                  An analogous report API that is scoped to a specific Environment.
            """
            ...

    summary: __summary_spec

class Environment(modal.object.Object):
    _name: typing.Optional[str]
    _settings: modal._environments.EnvironmentSettings

    def __init__(self):
        """mdmd:hidden"""
        ...

    @property
    def name(self) -> typing.Optional[str]: ...
    @synchronicity.classproperty
    @classmethod
    def objects(cls) -> EnvironmentManager: ...
    @property
    def roles(self) -> EnvironmentRolesManager:
        """Namespace with methods for managing the Environment Roles of users and service users.

        See https://modal.com/docs/guide/rbac for more information on Environment Roles.
        """
        ...

    @property
    def members(self) -> EnvironmentMembersManager:
        """mdmd:hidden"""
        ...

    def _hydrate_metadata(self, metadata: typing.Optional[google.protobuf.message.Message]): ...
    @staticmethod
    def _get_or_create(
        name: str, repr: str, create_if_missing: bool = False, client: typing.Optional[modal.client.Client] = None
    ) -> Environment: ...
    @staticmethod
    def from_context(*, client: typing.Optional[modal.client.Client] = None) -> Environment:
        """Look up an Environment object using the current context.

        This method returns the Environment that is defined by the local configuration
        (i.e., your active profile or the `MODAL_ENVIRONMENT` environment variable), or
        it fetches the default environment from the server when not defined locally.
        If called inside a Modal container, it will return the Environment that container
        is associated with.
        """
        ...

    @staticmethod
    def from_name(
        name: str, *, create_if_missing: bool = False, client: typing.Optional[modal.client.Client] = None
    ) -> Environment:
        """Look up an Environment object using its name."""
        ...

    @property
    def billing(self) -> EnvironmentBillingManager:
        """Namespace for Environment billing APIs."""
        ...

class __create_environment_spec(typing_extensions.Protocol):
    def __call__(self, /, name: str, client: typing.Optional[modal.client.Client] = None): ...
    async def aio(self, /, name: str, client: typing.Optional[modal.client.Client] = None): ...

create_environment: __create_environment_spec

class __delete_environment_spec(typing_extensions.Protocol):
    def __call__(self, /, name: str, client: typing.Optional[modal.client.Client] = None): ...
    async def aio(self, /, name: str, client: typing.Optional[modal.client.Client] = None): ...

delete_environment: __delete_environment_spec

class __list_environments_spec(typing_extensions.Protocol):
    def __call__(
        self, /, client: typing.Optional[modal.client.Client] = None
    ) -> list[modal_proto.api_pb2.EnvironmentListItem]: ...
    async def aio(
        self, /, client: typing.Optional[modal.client.Client] = None
    ) -> list[modal_proto.api_pb2.EnvironmentListItem]: ...

list_environments: __list_environments_spec

class __update_environment_spec(typing_extensions.Protocol):
    def __call__(
        self,
        /,
        current_name: str,
        *,
        new_name: typing.Optional[str] = None,
        new_web_suffix: typing.Optional[str] = None,
        client: typing.Optional[modal.client.Client] = None,
    ): ...
    async def aio(
        self,
        /,
        current_name: str,
        *,
        new_name: typing.Optional[str] = None,
        new_web_suffix: typing.Optional[str] = None,
        client: typing.Optional[modal.client.Client] = None,
    ): ...

update_environment: __update_environment_spec
