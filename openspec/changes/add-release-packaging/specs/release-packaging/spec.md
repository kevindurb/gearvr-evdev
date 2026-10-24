# Spec Delta

## Purpose

Defines the official build, contents, installation behavior, and publication
of driver release artifacts so that users and image builders can install a
known version without a Rust toolchain.

## ADDED Requirements

### Requirement: Release artifacts

Each published release SHALL provide a compressed archive and an RPM package
for x86_64 Linux, both carrying the same driver version as the release tag. A
checksum file SHALL accompany the artifacts.

#### Scenario: Release published

- **WHEN** a GitHub release is published
- **THEN** the compressed archive, the RPM, and a checksum file are attached to
  that release

#### Scenario: Artifact build fails

- **WHEN** either artifact fails to build or validate
- **THEN** no artifacts are attached to the release and the release job reports
  which artifact failed

#### Scenario: Checksums match

- **WHEN** a user downloads an artifact and the checksum file
- **THEN** the listed checksum matches the downloaded artifact

### Requirement: Portable binary baseline

The release binary SHALL run on x86_64 Linux systems with glibc 2.35 or newer
and the system D-Bus client library available. The binary MUST NOT bundle a
private copy of the D-Bus client library.

#### Scenario: Supported host

- **WHEN** the binary is run on a host meeting the glibc baseline with BlueZ
  installed
- **THEN** version and diagnostic commands run without a missing-library error

#### Scenario: Missing D-Bus client library

- **WHEN** the binary is run on a host without the system D-Bus client library
- **THEN** the dynamic loader names the missing library and the installation
  documentation identifies the package that provides it

#### Scenario: Older glibc

- **WHEN** the binary is run on a host older than the documented glibc baseline
- **THEN** installation documentation states that the host is unsupported and
  the release notes name the baseline

### Requirement: Archive installation

The compressed archive SHALL contain the driver binary, a systemd user unit,
udev rules, an installer, documentation, and the license. The installer SHALL
support a non-root per-user installation, SHALL not require the service to be
enabled by hand, and SHALL identify the exact privileged step needed to install
udev rules.

#### Scenario: Per-user install

- **WHEN** a user runs the documented per-user installation on a supported host
- **THEN** the binary is placed in a user executable directory, the user unit is
  installed and enabled, and the service starts

#### Scenario: Missing privileged udev step

- **WHEN** a user installs without root privileges
- **THEN** the installer prints the exact command needed to install the udev
  rules and the diagnostics report which device permissions are missing

#### Scenario: Repeated install

- **WHEN** the installer is run again for the same version
- **THEN** it completes without duplicating units or failing on existing files

#### Scenario: Uninstall

- **WHEN** the documented uninstall command is run
- **THEN** the user binary, unit, and enablement links are removed while user
  captures and calibration profiles are preserved

### Requirement: RPM installation layout

The RPM SHALL install the binary under `/usr/bin`, the user unit under
`/usr/lib/systemd/user`, and udev rules under `/usr/lib/udev/rules.d`. It SHALL
declare its runtime dependency on the system D-Bus client library and SHALL
NOT enable the user service automatically.

#### Scenario: Package installation

- **WHEN** the RPM is installed on a supported Fedora or bootc host
- **THEN** the binary, unit, and udev rules are present and the user service
  remains disabled until the user enables it

#### Scenario: Image build without a running systemd

- **WHEN** the RPM is installed while building a bootc image, where systemd and
  udev are not running
- **THEN** installation completes successfully and leaves no failed unit state

#### Scenario: Package removal

- **WHEN** the RPM is removed
- **THEN** the packaged binary, unit, and udev rules are removed and user
  configuration and captures remain

### Requirement: Input and device permissions

The release packages SHALL include udev rules that grant the logged-in user
access to the driver's input nodes and to the device nodes needed to create
them. Rules SHALL be ordered before the seat rules that consume the access tag.
Installation documentation SHALL state the security consequence of granting
HID-emulation access to every logged-in user.

#### Scenario: Fresh install

- **WHEN** a fresh supported installation creates the virtual gamepad and
  motion nodes
- **THEN** the logged-in user can open them without root and without an
  additional manual udev rule

#### Scenario: DualSense backend

- **WHEN** a fresh supported installation runs the DualSense backend
- **THEN** the logged-in user can open the virtual HID device without root

#### Scenario: Multi-user host

- **WHEN** the installation is used on a host with multiple logged-in users
- **THEN** the documentation identifies the HID-emulation permission tradeoff
  and how to restrict it

#### Scenario: Rule order

- **WHEN** the rules are installed with the documented filenames
- **THEN** they are processed before the seat rules that apply the access tag

### Requirement: Bootc enablement guidance

The installation documentation SHALL show how to install the RPM into a bootc
image and enable the user service globally at image build time. It SHALL state
that global enablement assumes a single interactive user.

#### Scenario: Bootc image

- **WHEN** an image builder follows the documented Containerfile steps
- **THEN** the driver is present in the image and the user service starts for
  the interactive user after login

#### Scenario: Multiple interactive users

- **WHEN** an image is expected to serve multiple interactive users
- **THEN** the documentation directs the builder to enable the service per user
  instead of globally

### Requirement: Release integrity checks

Release automation SHALL refuse to publish when the release tag does not match
the workspace version, and SHALL build with dependency resolution locked to the
committed lockfile.

#### Scenario: Matching version

- **WHEN** a `vMAJOR.MINOR.PATCH` release tag matches the workspace version
- **THEN** the release build proceeds and publishes artifacts whose reported
  version matches the tag

#### Scenario: Mismatched version

- **WHEN** the release tag does not match the workspace version
- **THEN** the release job fails before uploading assets and reports both values

#### Scenario: Unlocked dependency resolution

- **WHEN** a release build would change dependency versions from the committed
  lockfile
- **THEN** the build fails instead of producing a different dependency set

### Requirement: Continuous verification

The project SHALL run formatting, workspace tests, and lint checks for changes
proposed through the repository's normal review path, and the release build
SHALL use the same verification before publishing artifacts.

#### Scenario: Pull request checks

- **WHEN** a change is proposed
- **THEN** formatting, tests, and lint checks run and a failure blocks the
  release path

#### Scenario: Release verification

- **WHEN** a release is published
- **THEN** formatting, tests, and lint checks pass before artifacts are built
