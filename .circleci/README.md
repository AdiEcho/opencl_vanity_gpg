# Builds and releases

One Rust/Cargo project builds release binaries on three native executors:

| Job | Environment | Binary target | Package |
| --- | --- | --- | --- |
| `build-linux` | `cimg/rust:1.100` | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| `build-macos` | Xcode 27.1.0, `m4pro.medium` | `aarch64-apple-darwin` (Apple Silicon) | `.tar.gz` |
| `build-windows` | Windows Server 2022, `windows.medium` | `x86_64-pc-windows-msvc` | `.zip` |

All jobs use Rust 1.100.0 (the Linux minor tag can receive patch updates), fetch
locked dependencies, build with `cargo build --release --locked`, and smoke
test the binary with `--help`. Cargo downloads are cached by platform, Rust
minor version, and `Cargo.lock` checksum. The Linux job also runs the tests.

The Linux job runs `cargo test --locked` through two CircleCI Smarter Testing
suites. There is no existing lint command. Builds require no secrets or database.

There is no published Rust setup function. `setup-python@v0.5.0` uses the
Python already in the image for the standard-library-only JUnit adapter and
package script; no Python packages are needed. `circleci/rust@2.0.0` installs
Rust on macOS; `circleci/windows@5.1.1` provides the Windows executor. Windows
installs the MSVC Rust toolchain with rustup. No function caches are duplicated.

Linux still needs an OpenCL loader to link the binary, even for CPU-only tests.
The job builds Khronos OpenCL headers/loader release `v2026.05.29` using Git,
CMake, and the C compiler already in the image. It does not install a device
driver. The job sets `RUSTFLAGS=-L native=/usr/local/lib` so Rust's linker can
find the installed loader; `ldconfig` makes it available at runtime.
The five tests marked `#[ignore]` continue to be skipped because they
require an OpenCL device; the five ordinary tests run.

macOS links its system OpenCL framework. Windows downloads Khronos OpenCL SDK
`v2026.05.29`, verifies its SHA-256, and uses its x64 import library and loader
for the build and smoke test. Packages contain the binary, project license,
and README. Linux and Windows users still need an OpenCL loader and a device
driver; macOS packages use the system framework.

## Download artifacts

Every successful build writes its package and `.sha256` file to
`target/artifacts`. In CircleCI, open the pipeline, open a platform build job,
and select **Artifacts → binaries**. Each package name includes its target
triple. These are unsigned binaries.

## Automatic GitHub Releases

Push a version tag such as `v1.0.2` to run all three builds and then
`publish-release`. Ordinary branch pushes build and upload CircleCI artifacts
without creating a GitHub release. No new tag or release is created by adding
this configuration.

The release job waits for all three builds (including Linux tests), checks
the three packages and their checksums, then uploads all packages plus
`SHA256SUMS` to the tag's GitHub Release. It generates release notes for a new
release and marks tags containing `-` (such as `v1.0.2-rc.1`) as prereleases.
Reruns replace matching assets on an existing release and retain its notes.
The job uses `setup-github-cli@v0.3.0`.

Set **Project Settings → Environment Variables → GH_TOKEN** in CircleCI to a
GitHub token with access to this repository and **Contents: Read and write**.
The token is only needed by the release job. Configure the CircleCI project
trigger to receive Git tags as well as branch pushes. The CircleCI organization
must have access to Windows and macOS executors.

The adapter discovers tracked Rust files containing `#[test]`, filters unit
tests by their inline `mod tests` module, and runs integration test targets by
filename. If the crate gains different test layouts, update the adapter too.
Stable Rust's test runner cannot emit JUnit directly, so the adapter converts
its per-test statuses, preserves failures and skips, and checks the totals.
Reports are stored under `target/test-results` and uploaded for the Tests tab
and Test Insights. No suite options are enabled.

To validate locally (with Rust and the platform's OpenCL loader installed):

```sh
circleci testsuite doctor "Rust unit tests"
circleci testsuite doctor "CLI integration tests"
circleci config validate .circleci/config.yml
```

Connect this repository in CircleCI and select `.circleci/config.yml` as the
pipeline configuration if it is not already connected.
