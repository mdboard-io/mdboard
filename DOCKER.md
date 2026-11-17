# mdBoard Docker images

Docker Hub: https://hub.docker.com/repository/docker/mdboard/mdboard/general

## Image creation

Images are built by `.github/workflows/docker.yml` only from the public repository
`mdboard-io/mdboard`. The private repository does not publish to Docker Hub.

A single `prepare` job advances `BUILD`/`BUILD_DATE`, resolves the channel, and uploads
`VERSION`, `BUILD`, `BUILD_DATE` and `TAG`. Every build job consumes that one metadata
artifact. Build jobs must never call `prepare-build` independently.

| Platform | Runner | Image |
|---|---|---|
| `linux/amd64` | `ubuntu-24.04` | Debian slim |
| `linux/arm64` | `ubuntu-24.04-arm` | Debian slim |
| `windows/amd64` | `windows-2022` | `servercore:ltsc2022` |

There is no macOS image. macOS users run the Linux image through Docker Desktop.

## Channels

| Branch | Channel | Build in tag |
|---|---|---|
| `upstream` | `preview` | yes |
| `nightly` | `nightly` | no |
| `stable` | `stable` | no |
| other | `dev` | yes |

`stable` is the explicit mdBoard TAG value on the stable branch. It is no longer empty.

## Tags

Per-architecture:

```text
<os>-<arch>-<channel>-<version>[-<build>]
```

Multi-arch Linux:

```text
linux-<channel>-<version>[-<build>]
```

Examples:

```text
linux-amd64-stable-0.1990.7
linux-arm64-nightly-0.1990.7
linux-amd64-preview-0.1990.7-20261114-19
linux-stable-0.1990.7
linux-nightly-0.1990.7
windows-amd64-stable-0.1990.7
```

Windows publishes only `windows/amd64` tags. No `windows-*` multi-arch tag exists until a
second Windows architecture or a deliberate single-platform manifest policy is added.

## `latest`

Only channel-scoped `latest` tags are published. There is no global `latest`.

```text
linux-amd64-nightly-latest
linux-arm64-nightly-latest
linux-amd64-stable-latest
linux-arm64-stable-latest
linux-nightly-latest
linux-stable-latest
windows-amd64-nightly-latest
windows-amd64-stable-latest
```

No `latest` for preview or dev. No Windows multi-arch latest while only `windows/amd64`
exists.

Stable and nightly rebuilds overwrite the same version and latest tags. This is intentional.

## Metadata

Images carry OCI labels:

```text
org.opencontainers.image.version
org.opencontainers.image.revision
org.opencontainers.image.created
org.opencontainers.image.source
org.opencontainers.image.title
org.opencontainers.image.licenses
io.mdboard.build
io.mdboard.channel
```

`revision` is the full Git commit. `io.mdboard.build` is the build number. `version` is the
strict SemVer version without a build suffix.

Linux multi-arch manifests also attempt OCI index annotations for version, revision, created,
build and channel. If Docker Hub rejects index annotations, the manifest still publishes and
image labels remain authoritative.

## Windows limitations

The Windows image:

- uses Windows Server 2022 / `ltsc2022`;
- requires a matching `windows-2022` runner;
- is built from a precompiled `x86_64-pc-windows-msvc` binary;
- includes MinGit for Git-backed mdBoard operations;
- is not interchangeable with the Linux image.

Windows Server 2025 support is a future addition.

## Existing tags

Existing Docker Hub tags are removed by the owner. No compatibility aliases are published.
CI does not delete registry tags.
