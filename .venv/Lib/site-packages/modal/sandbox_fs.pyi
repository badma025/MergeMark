import modal.sandbox
import modal.types
import os
import typing
import typing_extensions

def _log_throughput(op: str, size_bytes: int, dur_s: float) -> None: ...
def _expand_watch_filter(filter: list[modal.types.FileWatchEventType]) -> list[str]:
    """Expand a Python filter list into modal-sandbox-fs-tools event type strings.

    FileWatchEventType.Modify covers fs tool's Rename/RenameFrom/RenameTo variants,
    so those must be included when the caller filters for Modify events.
    """
    ...

class _SandboxFilesystem:
    """mdmd:namespace"""

    _container: typing.Union[modal.sandbox._Sandbox, modal.sandbox._SidecarContainer]

    def __init__(self, container: typing.Union[modal.sandbox._Sandbox, modal.sandbox._SidecarContainer]) -> None:
        """mdmd:hidden"""
        ...

    async def copy_from_local(self, local_path: typing.Union[str, os.PathLike], remote_path: str) -> None:
        """Copy a local file into the Sandbox.

        `remote_path` must be an absolute path to a file in the Sandbox.
        Parent directories for `remote_path` are created if needed.
        The remote file is overwritten if it already exists.

        Args:
            local_path: Path to the file on the local machine.
            remote_path: Absolute path to the file in the Sandbox.

        Raises:
            SandboxFilesystemNotADirectoryError: A parent path component of ``remote_path`` is not a directory.
            SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
            SandboxFilesystemPermissionError: Write permission is denied in the Sandbox.
            SandboxFilesystemError: The command fails for any other reason.
            FileNotFoundError: ``local_path`` does not exist.
            IsADirectoryError: ``local_path`` is a directory.
            PermissionError: Reading ``local_path`` is not permitted.

        Examples:
            ```python fixture:sandbox fixture:tmpdir
            import tempfile
            from pathlib import Path

            local_path = Path(tempfile.mktemp())
            local_path.write_text("Hello, world!\n")
            sandbox.filesystem.copy_from_local(local_path, "/tmp/hello.txt")
            ```
        """
        ...

    async def copy_to_local(self, remote_path: str, local_path: typing.Union[str, os.PathLike]) -> None:
        """Copy a file from the Sandbox to a local path.

        `remote_path` must be an absolute path to a file in the Sandbox.
        Parent directories for `local_path` are created if needed.
        The local file is overwritten if it already exists.

        **Raises**

        - `SandboxFilesystemNotFoundError`: the remote path does not exist.
        - `SandboxFilesystemIsADirectoryError`: the remote path points to a directory.
        - `SandboxFilesystemPermissionError`: read permission is denied in the Sandbox.
        - `SandboxFilesystemError`: the command fails for any other reason.
        - `IsADirectoryError`: `local_path` points to a directory.
        - `NotADirectoryError`: a component of the `local_path` parent is not a directory.
        - `PermissionError`: writing `local_path` is not permitted.

        **Usage**

        ```python fixture:sandbox fixture:tmpdir
        sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
        sandbox.filesystem.copy_to_local("/tmp/hello.txt", "/tmp/local-hello.txt")
        ```
        """
        ...

    async def list_files(self, remote_path: str) -> list[modal.types.FileInfo]:
        """List files and directories in a Sandbox directory.

        Args:
            remote_path: Absolute path to the directory in the Sandbox.

        Returns:
            A list of `FileInfo` objects describing each entry.

        Raises:
            SandboxFilesystemNotFoundError: The path does not exist.
            SandboxFilesystemNotADirectoryError: The path is not a directory.
            SandboxFilesystemPermissionError: Read permission is denied.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            ```python fixture:sandbox
            entries = sandbox.filesystem.list_files("/tmp")
            for entry in entries:
                print(entry.name, entry.type, entry.size)
            ```
        """
        ...

    async def make_directory(self, remote_path: str, *, create_parents: bool = True) -> None:
        """Create a new directory in the Sandbox.

        `remote_path` must be an absolute path in the Sandbox.

        When `create_parents` is `True` (the default), any missing parent directories are created and the call is
        idempotent (succeeds silently if the directory already exists). When `create_parents` is `False`, the
        immediate parent directory must already exist and the path must not already exist.

        Args:
            remote_path: Absolute path of the directory to create in the Sandbox.
            create_parents: When ``True``, create missing parents and succeed if the directory already exists.

        Raises:
            SandboxFilesystemNotFoundError: The parent directory does not exist and ``create_parents`` is false.
            SandboxFilesystemPathAlreadyExistsError: The path already exists.
            SandboxFilesystemNotADirectoryError: A path component is not a directory.
            SandboxFilesystemPermissionError: Creation is not permitted.
            InvalidError: The operation is not supported by the mount.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            ```python fixture:sandbox
            sandbox.filesystem.make_directory("/tmp/a/b/c")
            ```
        """
        ...

    async def read_bytes(self, remote_path: str) -> bytes:
        """Read a file from the Sandbox and return its contents as bytes.

        `remote_path` must be an absolute path to a file in the Sandbox.

        Args:
            remote_path: Absolute path to the file in the Sandbox.

        Returns:
            Raw bytes read from the file.

        Raises:
            SandboxFilesystemNotFoundError: The path does not exist.
            SandboxFilesystemIsADirectoryError: The path points to a directory.
            SandboxFilesystemPermissionError: Read permission is denied.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            ```python fixture:sandbox
            sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
            contents = sandbox.filesystem.read_bytes("/tmp/hello.bin")
            print(contents.decode("utf-8"))
            ```
        """
        ...

    async def read_text(self, remote_path: str) -> str:
        """Read a file from the Sandbox and return its contents as a UTF-8 string.

        `remote_path` must be an absolute path to a file in the Sandbox.

        Args:
            remote_path: Absolute path to the file in the Sandbox.

        Returns:
            File contents decoded as UTF-8.

        Raises:
            SandboxFilesystemNotFoundError: The path does not exist.
            SandboxFilesystemIsADirectoryError: The path points to a directory.
            SandboxFilesystemPermissionError: Read permission is denied.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            ```python fixture:sandbox
            sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
            contents = sandbox.filesystem.read_text("/tmp/hello.txt")
            print(contents)
            ```
        """
        ...

    async def remove(self, remote_path: str, *, recursive: bool = False) -> None:
        """Remove a file or directory in the Sandbox.

        When `remote_path` is a directory and `recursive` is `False` (the
        default), removes it only if it is empty. When `recursive` is `True`,
        removes the directory and all its contents.

        Recursive directory removal is not supported on all mounts.
        In particular, `CloudBucketMount` does not support it. An
        `InvalidError` is raised in that case.

        Args:
            remote_path: Absolute path to the file in the Sandbox.
            recursive: When ``True``, remove the directory and all its contents.

        Raises:
            SandboxFilesystemNotFoundError: The remote path does not exist.
            SandboxFilesystemDirectoryNotEmptyError: `recursive` is `False` and the directory is not empty.
            SandboxFilesystemPermissionError: Read permission is denied in the Sandbox.
            InvalidError: The operation is not supported by the mount.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            To remove a file:

            ```python fixture:sandbox
            sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
            sandbox.filesystem.remove("/tmp/hello.bin")
            ```

            To remove a directory and all its contents:

            ```python fixture:sandbox
            sandbox.filesystem.make_directory("/tmp/mydir/subdir")
            sandbox.filesystem.remove("/tmp/mydir", recursive=True)
            ```
        """
        ...

    async def stat(self, remote_path: str) -> modal.types.FileInfo:
        """Return metadata for a single file, directory, or symlink in the Sandbox.

        `remote_path` must be an absolute path in the Sandbox. If `remote_path` is a symlink, the returned
        `FileInfo` object describes the symlink, not the target it points to.

        **Raises**

        - `SandboxFilesystemNotFoundError`: the path does not exist.
        - `SandboxFilesystemNotADirectoryError`: a non-leaf component of the path is not a directory.
        - `SandboxFilesystemPermissionError`: a component of the path is not searchable.
        - `SandboxFilesystemError`: the command fails for any other reason.

        **Usage**

        ```python fixture:sandbox
        sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
        info = sandbox.filesystem.stat("/tmp/hello.txt")
        print(info.size, info.permissions, info.modified_time)
        ```
        """
        ...

    def watch(
        self,
        remote_path: str,
        *,
        filter: typing.Optional[list[modal.types.FileWatchEventType]] = None,
        recursive: bool = False,
        timeout: typing.Optional[int] = None,
    ) -> typing.AsyncIterator[modal.types.FileWatchEvent]:
        """Watch a path in the Sandbox for filesystem changes.

        `remote_path` must be an absolute path in the Sandbox. If it points
        to a file, events for that file are reported. If it points to a
        directory, events for entries directly inside it are reported. Set
        `recursive=True` to also receive events for all nested subdirectories.
        If `remote_path` is a symlink, it is followed and events reference
        paths under the resolved target.

        Yields `FileWatchEvent` objects as changes occur, until either
        `timeout` seconds elapse, the iterator is closed, or the Sandbox
        is terminated.

        Optionally restrict the kinds of events emitted to those included
        in `filter`. The default filter `None` permits all event types.

        `timeout` is in seconds. `None` means watch indefinitely. When
        `timeout` elapses, the iterator stops without raising an exception.

        **Raises**

        - `SandboxFilesystemNotFoundError`: `remote_path` does not exist.
        - `SandboxFilesystemPermissionError`: watch access is denied.
        - `InvalidError`: the filesystem at `remote_path` does not support
          watching.
        - `SandboxFilesystemError`: the command fails for any other reason.

        **Usage**

        ```python notest
        for event in sandbox.filesystem.watch(
            "/tmp/foo",
            recursive=True,
            filter=[FileWatchEventType.Create],
            timeout=60,
        ):
            if any(p.endswith(".done") for p in event.paths):
                break
        ```
        """
        ...

    async def write_bytes(self, data: typing.Union[bytes, bytearray, memoryview], remote_path: str) -> None:
        """Write binary content to a file in the Sandbox.

        `remote_path` must be an absolute path to a file in the Sandbox.
        Parent directories for `remote_path` are created if needed.
        The remote file is overwritten if it already exists.

        Args:
            data: Bytes to write.
            remote_path: Absolute path to the file in the Sandbox.

        Raises:
            TypeError: ``data`` is not bytes-like.
            SandboxFilesystemNotADirectoryError: A parent path component is not a directory.
            SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
            SandboxFilesystemPermissionError: Write permission is denied.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            ```python fixture:sandbox
            sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
            ```
        """
        ...

    async def write_text(self, data: str, remote_path: str) -> None:
        """Write UTF-8 text to a file in the Sandbox.

        `remote_path` must be an absolute path to a file in the Sandbox.
        Parent directories for `remote_path` are created if needed.
        The remote file is overwritten if it already exists.

        Args:
            data: Text to write (encoded as UTF-8).
            remote_path: Absolute path to the file in the Sandbox.

        Raises:
            TypeError: ``data`` is not a string.
            SandboxFilesystemNotADirectoryError: A parent path component is not a directory.
            SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
            SandboxFilesystemPermissionError: Write permission is denied.
            SandboxFilesystemError: The command fails for any other reason.

        Examples:
            ```python fixture:sandbox
            sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
            ```
        """
        ...

    async def _exec_fs_tool_write(self, source: typing.BinaryIO, remote_path: str, op_name: str) -> int:
        """Exec the FS-tools write command and stream `source` into its stdin.

        Returns the number of bytes streamed.
        """
        ...

class SandboxFilesystem:
    """mdmd:namespace"""

    _container: typing.Union[modal.sandbox.Sandbox, modal.sandbox.SidecarContainer]

    def __init__(self, container: typing.Union[modal.sandbox.Sandbox, modal.sandbox.SidecarContainer]) -> None:
        """mdmd:hidden"""
        ...

    class __copy_from_local_spec(typing_extensions.Protocol):
        def __call__(self, /, local_path: typing.Union[str, os.PathLike], remote_path: str) -> None:
            """Copy a local file into the Sandbox.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `remote_path` are created if needed.
            The remote file is overwritten if it already exists.

            Args:
                local_path: Path to the file on the local machine.
                remote_path: Absolute path to the file in the Sandbox.

            Raises:
                SandboxFilesystemNotADirectoryError: A parent path component of ``remote_path`` is not a directory.
                SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
                SandboxFilesystemPermissionError: Write permission is denied in the Sandbox.
                SandboxFilesystemError: The command fails for any other reason.
                FileNotFoundError: ``local_path`` does not exist.
                IsADirectoryError: ``local_path`` is a directory.
                PermissionError: Reading ``local_path`` is not permitted.

            Examples:
                ```python fixture:sandbox fixture:tmpdir
                import tempfile
                from pathlib import Path

                local_path = Path(tempfile.mktemp())
                local_path.write_text("Hello, world!\n")
                sandbox.filesystem.copy_from_local(local_path, "/tmp/hello.txt")
                ```
            """
            ...

        async def aio(self, /, local_path: typing.Union[str, os.PathLike], remote_path: str) -> None:
            """Copy a local file into the Sandbox.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `remote_path` are created if needed.
            The remote file is overwritten if it already exists.

            Args:
                local_path: Path to the file on the local machine.
                remote_path: Absolute path to the file in the Sandbox.

            Raises:
                SandboxFilesystemNotADirectoryError: A parent path component of ``remote_path`` is not a directory.
                SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
                SandboxFilesystemPermissionError: Write permission is denied in the Sandbox.
                SandboxFilesystemError: The command fails for any other reason.
                FileNotFoundError: ``local_path`` does not exist.
                IsADirectoryError: ``local_path`` is a directory.
                PermissionError: Reading ``local_path`` is not permitted.

            Examples:
                ```python fixture:sandbox fixture:tmpdir
                import tempfile
                from pathlib import Path

                local_path = Path(tempfile.mktemp())
                local_path.write_text("Hello, world!\n")
                sandbox.filesystem.copy_from_local(local_path, "/tmp/hello.txt")
                ```
            """
            ...

    copy_from_local: __copy_from_local_spec

    class __copy_to_local_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str, local_path: typing.Union[str, os.PathLike]) -> None:
            """Copy a file from the Sandbox to a local path.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `local_path` are created if needed.
            The local file is overwritten if it already exists.

            **Raises**

            - `SandboxFilesystemNotFoundError`: the remote path does not exist.
            - `SandboxFilesystemIsADirectoryError`: the remote path points to a directory.
            - `SandboxFilesystemPermissionError`: read permission is denied in the Sandbox.
            - `SandboxFilesystemError`: the command fails for any other reason.
            - `IsADirectoryError`: `local_path` points to a directory.
            - `NotADirectoryError`: a component of the `local_path` parent is not a directory.
            - `PermissionError`: writing `local_path` is not permitted.

            **Usage**

            ```python fixture:sandbox fixture:tmpdir
            sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
            sandbox.filesystem.copy_to_local("/tmp/hello.txt", "/tmp/local-hello.txt")
            ```
            """
            ...

        async def aio(self, /, remote_path: str, local_path: typing.Union[str, os.PathLike]) -> None:
            """Copy a file from the Sandbox to a local path.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `local_path` are created if needed.
            The local file is overwritten if it already exists.

            **Raises**

            - `SandboxFilesystemNotFoundError`: the remote path does not exist.
            - `SandboxFilesystemIsADirectoryError`: the remote path points to a directory.
            - `SandboxFilesystemPermissionError`: read permission is denied in the Sandbox.
            - `SandboxFilesystemError`: the command fails for any other reason.
            - `IsADirectoryError`: `local_path` points to a directory.
            - `NotADirectoryError`: a component of the `local_path` parent is not a directory.
            - `PermissionError`: writing `local_path` is not permitted.

            **Usage**

            ```python fixture:sandbox fixture:tmpdir
            sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
            sandbox.filesystem.copy_to_local("/tmp/hello.txt", "/tmp/local-hello.txt")
            ```
            """
            ...

    copy_to_local: __copy_to_local_spec

    class __list_files_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str) -> list[modal.types.FileInfo]:
            """List files and directories in a Sandbox directory.

            Args:
                remote_path: Absolute path to the directory in the Sandbox.

            Returns:
                A list of `FileInfo` objects describing each entry.

            Raises:
                SandboxFilesystemNotFoundError: The path does not exist.
                SandboxFilesystemNotADirectoryError: The path is not a directory.
                SandboxFilesystemPermissionError: Read permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                entries = sandbox.filesystem.list_files("/tmp")
                for entry in entries:
                    print(entry.name, entry.type, entry.size)
                ```
            """
            ...

        async def aio(self, /, remote_path: str) -> list[modal.types.FileInfo]:
            """List files and directories in a Sandbox directory.

            Args:
                remote_path: Absolute path to the directory in the Sandbox.

            Returns:
                A list of `FileInfo` objects describing each entry.

            Raises:
                SandboxFilesystemNotFoundError: The path does not exist.
                SandboxFilesystemNotADirectoryError: The path is not a directory.
                SandboxFilesystemPermissionError: Read permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                entries = sandbox.filesystem.list_files("/tmp")
                for entry in entries:
                    print(entry.name, entry.type, entry.size)
                ```
            """
            ...

    list_files: __list_files_spec

    class __make_directory_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str, *, create_parents: bool = True) -> None:
            """Create a new directory in the Sandbox.

            `remote_path` must be an absolute path in the Sandbox.

            When `create_parents` is `True` (the default), any missing parent directories are created and the call is
            idempotent (succeeds silently if the directory already exists). When `create_parents` is `False`, the
            immediate parent directory must already exist and the path must not already exist.

            Args:
                remote_path: Absolute path of the directory to create in the Sandbox.
                create_parents: When ``True``, create missing parents and succeed if the directory already exists.

            Raises:
                SandboxFilesystemNotFoundError: The parent directory does not exist and ``create_parents`` is false.
                SandboxFilesystemPathAlreadyExistsError: The path already exists.
                SandboxFilesystemNotADirectoryError: A path component is not a directory.
                SandboxFilesystemPermissionError: Creation is not permitted.
                InvalidError: The operation is not supported by the mount.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.make_directory("/tmp/a/b/c")
                ```
            """
            ...

        async def aio(self, /, remote_path: str, *, create_parents: bool = True) -> None:
            """Create a new directory in the Sandbox.

            `remote_path` must be an absolute path in the Sandbox.

            When `create_parents` is `True` (the default), any missing parent directories are created and the call is
            idempotent (succeeds silently if the directory already exists). When `create_parents` is `False`, the
            immediate parent directory must already exist and the path must not already exist.

            Args:
                remote_path: Absolute path of the directory to create in the Sandbox.
                create_parents: When ``True``, create missing parents and succeed if the directory already exists.

            Raises:
                SandboxFilesystemNotFoundError: The parent directory does not exist and ``create_parents`` is false.
                SandboxFilesystemPathAlreadyExistsError: The path already exists.
                SandboxFilesystemNotADirectoryError: A path component is not a directory.
                SandboxFilesystemPermissionError: Creation is not permitted.
                InvalidError: The operation is not supported by the mount.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.make_directory("/tmp/a/b/c")
                ```
            """
            ...

    make_directory: __make_directory_spec

    class __read_bytes_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str) -> bytes:
            """Read a file from the Sandbox and return its contents as bytes.

            `remote_path` must be an absolute path to a file in the Sandbox.

            Args:
                remote_path: Absolute path to the file in the Sandbox.

            Returns:
                Raw bytes read from the file.

            Raises:
                SandboxFilesystemNotFoundError: The path does not exist.
                SandboxFilesystemIsADirectoryError: The path points to a directory.
                SandboxFilesystemPermissionError: Read permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
                contents = sandbox.filesystem.read_bytes("/tmp/hello.bin")
                print(contents.decode("utf-8"))
                ```
            """
            ...

        async def aio(self, /, remote_path: str) -> bytes:
            """Read a file from the Sandbox and return its contents as bytes.

            `remote_path` must be an absolute path to a file in the Sandbox.

            Args:
                remote_path: Absolute path to the file in the Sandbox.

            Returns:
                Raw bytes read from the file.

            Raises:
                SandboxFilesystemNotFoundError: The path does not exist.
                SandboxFilesystemIsADirectoryError: The path points to a directory.
                SandboxFilesystemPermissionError: Read permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
                contents = sandbox.filesystem.read_bytes("/tmp/hello.bin")
                print(contents.decode("utf-8"))
                ```
            """
            ...

    read_bytes: __read_bytes_spec

    class __read_text_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str) -> str:
            """Read a file from the Sandbox and return its contents as a UTF-8 string.

            `remote_path` must be an absolute path to a file in the Sandbox.

            Args:
                remote_path: Absolute path to the file in the Sandbox.

            Returns:
                File contents decoded as UTF-8.

            Raises:
                SandboxFilesystemNotFoundError: The path does not exist.
                SandboxFilesystemIsADirectoryError: The path points to a directory.
                SandboxFilesystemPermissionError: Read permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
                contents = sandbox.filesystem.read_text("/tmp/hello.txt")
                print(contents)
                ```
            """
            ...

        async def aio(self, /, remote_path: str) -> str:
            """Read a file from the Sandbox and return its contents as a UTF-8 string.

            `remote_path` must be an absolute path to a file in the Sandbox.

            Args:
                remote_path: Absolute path to the file in the Sandbox.

            Returns:
                File contents decoded as UTF-8.

            Raises:
                SandboxFilesystemNotFoundError: The path does not exist.
                SandboxFilesystemIsADirectoryError: The path points to a directory.
                SandboxFilesystemPermissionError: Read permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
                contents = sandbox.filesystem.read_text("/tmp/hello.txt")
                print(contents)
                ```
            """
            ...

    read_text: __read_text_spec

    class __remove_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str, *, recursive: bool = False) -> None:
            """Remove a file or directory in the Sandbox.

            When `remote_path` is a directory and `recursive` is `False` (the
            default), removes it only if it is empty. When `recursive` is `True`,
            removes the directory and all its contents.

            Recursive directory removal is not supported on all mounts.
            In particular, `CloudBucketMount` does not support it. An
            `InvalidError` is raised in that case.

            Args:
                remote_path: Absolute path to the file in the Sandbox.
                recursive: When ``True``, remove the directory and all its contents.

            Raises:
                SandboxFilesystemNotFoundError: The remote path does not exist.
                SandboxFilesystemDirectoryNotEmptyError: `recursive` is `False` and the directory is not empty.
                SandboxFilesystemPermissionError: Read permission is denied in the Sandbox.
                InvalidError: The operation is not supported by the mount.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                To remove a file:

                ```python fixture:sandbox
                sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
                sandbox.filesystem.remove("/tmp/hello.bin")
                ```

                To remove a directory and all its contents:

                ```python fixture:sandbox
                sandbox.filesystem.make_directory("/tmp/mydir/subdir")
                sandbox.filesystem.remove("/tmp/mydir", recursive=True)
                ```
            """
            ...

        async def aio(self, /, remote_path: str, *, recursive: bool = False) -> None:
            """Remove a file or directory in the Sandbox.

            When `remote_path` is a directory and `recursive` is `False` (the
            default), removes it only if it is empty. When `recursive` is `True`,
            removes the directory and all its contents.

            Recursive directory removal is not supported on all mounts.
            In particular, `CloudBucketMount` does not support it. An
            `InvalidError` is raised in that case.

            Args:
                remote_path: Absolute path to the file in the Sandbox.
                recursive: When ``True``, remove the directory and all its contents.

            Raises:
                SandboxFilesystemNotFoundError: The remote path does not exist.
                SandboxFilesystemDirectoryNotEmptyError: `recursive` is `False` and the directory is not empty.
                SandboxFilesystemPermissionError: Read permission is denied in the Sandbox.
                InvalidError: The operation is not supported by the mount.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                To remove a file:

                ```python fixture:sandbox
                sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
                sandbox.filesystem.remove("/tmp/hello.bin")
                ```

                To remove a directory and all its contents:

                ```python fixture:sandbox
                sandbox.filesystem.make_directory("/tmp/mydir/subdir")
                sandbox.filesystem.remove("/tmp/mydir", recursive=True)
                ```
            """
            ...

    remove: __remove_spec

    class __stat_spec(typing_extensions.Protocol):
        def __call__(self, /, remote_path: str) -> modal.types.FileInfo:
            """Return metadata for a single file, directory, or symlink in the Sandbox.

            `remote_path` must be an absolute path in the Sandbox. If `remote_path` is a symlink, the returned
            `FileInfo` object describes the symlink, not the target it points to.

            **Raises**

            - `SandboxFilesystemNotFoundError`: the path does not exist.
            - `SandboxFilesystemNotADirectoryError`: a non-leaf component of the path is not a directory.
            - `SandboxFilesystemPermissionError`: a component of the path is not searchable.
            - `SandboxFilesystemError`: the command fails for any other reason.

            **Usage**

            ```python fixture:sandbox
            sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
            info = sandbox.filesystem.stat("/tmp/hello.txt")
            print(info.size, info.permissions, info.modified_time)
            ```
            """
            ...

        async def aio(self, /, remote_path: str) -> modal.types.FileInfo:
            """Return metadata for a single file, directory, or symlink in the Sandbox.

            `remote_path` must be an absolute path in the Sandbox. If `remote_path` is a symlink, the returned
            `FileInfo` object describes the symlink, not the target it points to.

            **Raises**

            - `SandboxFilesystemNotFoundError`: the path does not exist.
            - `SandboxFilesystemNotADirectoryError`: a non-leaf component of the path is not a directory.
            - `SandboxFilesystemPermissionError`: a component of the path is not searchable.
            - `SandboxFilesystemError`: the command fails for any other reason.

            **Usage**

            ```python fixture:sandbox
            sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
            info = sandbox.filesystem.stat("/tmp/hello.txt")
            print(info.size, info.permissions, info.modified_time)
            ```
            """
            ...

    stat: __stat_spec

    class __watch_spec(typing_extensions.Protocol):
        def __call__(
            self,
            /,
            remote_path: str,
            *,
            filter: typing.Optional[list[modal.types.FileWatchEventType]] = None,
            recursive: bool = False,
            timeout: typing.Optional[int] = None,
        ) -> typing.Iterator[modal.types.FileWatchEvent]:
            """Watch a path in the Sandbox for filesystem changes.

            `remote_path` must be an absolute path in the Sandbox. If it points
            to a file, events for that file are reported. If it points to a
            directory, events for entries directly inside it are reported. Set
            `recursive=True` to also receive events for all nested subdirectories.
            If `remote_path` is a symlink, it is followed and events reference
            paths under the resolved target.

            Yields `FileWatchEvent` objects as changes occur, until either
            `timeout` seconds elapse, the iterator is closed, or the Sandbox
            is terminated.

            Optionally restrict the kinds of events emitted to those included
            in `filter`. The default filter `None` permits all event types.

            `timeout` is in seconds. `None` means watch indefinitely. When
            `timeout` elapses, the iterator stops without raising an exception.

            **Raises**

            - `SandboxFilesystemNotFoundError`: `remote_path` does not exist.
            - `SandboxFilesystemPermissionError`: watch access is denied.
            - `InvalidError`: the filesystem at `remote_path` does not support
              watching.
            - `SandboxFilesystemError`: the command fails for any other reason.

            **Usage**

            ```python notest
            for event in sandbox.filesystem.watch(
                "/tmp/foo",
                recursive=True,
                filter=[FileWatchEventType.Create],
                timeout=60,
            ):
                if any(p.endswith(".done") for p in event.paths):
                    break
            ```
            """
            ...

        def aio(
            self,
            /,
            remote_path: str,
            *,
            filter: typing.Optional[list[modal.types.FileWatchEventType]] = None,
            recursive: bool = False,
            timeout: typing.Optional[int] = None,
        ) -> typing.AsyncIterator[modal.types.FileWatchEvent]:
            """Watch a path in the Sandbox for filesystem changes.

            `remote_path` must be an absolute path in the Sandbox. If it points
            to a file, events for that file are reported. If it points to a
            directory, events for entries directly inside it are reported. Set
            `recursive=True` to also receive events for all nested subdirectories.
            If `remote_path` is a symlink, it is followed and events reference
            paths under the resolved target.

            Yields `FileWatchEvent` objects as changes occur, until either
            `timeout` seconds elapse, the iterator is closed, or the Sandbox
            is terminated.

            Optionally restrict the kinds of events emitted to those included
            in `filter`. The default filter `None` permits all event types.

            `timeout` is in seconds. `None` means watch indefinitely. When
            `timeout` elapses, the iterator stops without raising an exception.

            **Raises**

            - `SandboxFilesystemNotFoundError`: `remote_path` does not exist.
            - `SandboxFilesystemPermissionError`: watch access is denied.
            - `InvalidError`: the filesystem at `remote_path` does not support
              watching.
            - `SandboxFilesystemError`: the command fails for any other reason.

            **Usage**

            ```python notest
            for event in sandbox.filesystem.watch(
                "/tmp/foo",
                recursive=True,
                filter=[FileWatchEventType.Create],
                timeout=60,
            ):
                if any(p.endswith(".done") for p in event.paths):
                    break
            ```
            """
            ...

    watch: __watch_spec

    class __write_bytes_spec(typing_extensions.Protocol):
        def __call__(self, /, data: typing.Union[bytes, bytearray, memoryview], remote_path: str) -> None:
            """Write binary content to a file in the Sandbox.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `remote_path` are created if needed.
            The remote file is overwritten if it already exists.

            Args:
                data: Bytes to write.
                remote_path: Absolute path to the file in the Sandbox.

            Raises:
                TypeError: ``data`` is not bytes-like.
                SandboxFilesystemNotADirectoryError: A parent path component is not a directory.
                SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
                SandboxFilesystemPermissionError: Write permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
                ```
            """
            ...

        async def aio(self, /, data: typing.Union[bytes, bytearray, memoryview], remote_path: str) -> None:
            """Write binary content to a file in the Sandbox.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `remote_path` are created if needed.
            The remote file is overwritten if it already exists.

            Args:
                data: Bytes to write.
                remote_path: Absolute path to the file in the Sandbox.

            Raises:
                TypeError: ``data`` is not bytes-like.
                SandboxFilesystemNotADirectoryError: A parent path component is not a directory.
                SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
                SandboxFilesystemPermissionError: Write permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_bytes(b"Hello, world!\n", "/tmp/hello.bin")
                ```
            """
            ...

    write_bytes: __write_bytes_spec

    class __write_text_spec(typing_extensions.Protocol):
        def __call__(self, /, data: str, remote_path: str) -> None:
            """Write UTF-8 text to a file in the Sandbox.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `remote_path` are created if needed.
            The remote file is overwritten if it already exists.

            Args:
                data: Text to write (encoded as UTF-8).
                remote_path: Absolute path to the file in the Sandbox.

            Raises:
                TypeError: ``data`` is not a string.
                SandboxFilesystemNotADirectoryError: A parent path component is not a directory.
                SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
                SandboxFilesystemPermissionError: Write permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
                ```
            """
            ...

        async def aio(self, /, data: str, remote_path: str) -> None:
            """Write UTF-8 text to a file in the Sandbox.

            `remote_path` must be an absolute path to a file in the Sandbox.
            Parent directories for `remote_path` are created if needed.
            The remote file is overwritten if it already exists.

            Args:
                data: Text to write (encoded as UTF-8).
                remote_path: Absolute path to the file in the Sandbox.

            Raises:
                TypeError: ``data`` is not a string.
                SandboxFilesystemNotADirectoryError: A parent path component is not a directory.
                SandboxFilesystemIsADirectoryError: ``remote_path`` points to a directory.
                SandboxFilesystemPermissionError: Write permission is denied.
                SandboxFilesystemError: The command fails for any other reason.

            Examples:
                ```python fixture:sandbox
                sandbox.filesystem.write_text("Hello, world!\n", "/tmp/hello.txt")
                ```
            """
            ...

    write_text: __write_text_spec

    class ___exec_fs_tool_write_spec(typing_extensions.Protocol):
        def __call__(self, /, source: typing.BinaryIO, remote_path: str, op_name: str) -> int:
            """Exec the FS-tools write command and stream `source` into its stdin.

            Returns the number of bytes streamed.
            """
            ...

        async def aio(self, /, source: typing.BinaryIO, remote_path: str, op_name: str) -> int:
            """Exec the FS-tools write command and stream `source` into its stdin.

            Returns the number of bytes streamed.
            """
            ...

    _exec_fs_tool_write: ___exec_fs_tool_write_spec
