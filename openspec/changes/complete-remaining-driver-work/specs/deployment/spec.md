# Spec Delta

## Purpose

Make the driver installable and operable as an unprivileged user service with
correct input permissions, predictable restart behavior, and recovery guidance.

## ADDED Requirements

### Requirement: Input device permissions

The packaging SHALL provide a udev rule that grants the logged-in user access to
both virtual input nodes. The rule SHALL be ordered before the seat rules that
consume the `uaccess` tag.

#### Scenario: Gamepad access

- **WHEN** the gamepad node is created
- **THEN** the logged-in user can open it without root

#### Scenario: Motion node access

- **WHEN** the motion node is created
- **THEN** the udev rule applies `uaccess` before seat assignment and the
  logged-in user can read it

#### Scenario: Rule order

- **WHEN** the rule is installed with the documented filename
- **THEN** it sorts before `71-seat.rules` and the ACL is applied

### Requirement: Unprivileged user service

The packaging SHALL provide a systemd user unit that runs the driver without
root, starts after Bluetooth is available, and restarts after a failed session.
The unit SHALL write logs to the user journal and SHALL not require a graphical
desktop session beyond access to the user D-Bus and input permissions.

#### Scenario: Start at login

- **WHEN** the user service is enabled and the user logs in
- **THEN** the driver starts and begins waiting for or connecting to the
  controller

#### Scenario: Recover after failure

- **WHEN** the driver exits because the BLE session failed
- **THEN** systemd restarts it after the configured delay without manual action

#### Scenario: Clean stop

- **WHEN** the user stops the service
- **THEN** the driver releases held inputs, stops notifications, and exits
  without leaving a wedged virtual device

### Requirement: Pairing and recovery guidance

The documentation SHALL describe pairing, trusting, connecting, removing a
wedged bond, and verifying permissions for BlueZ, `/dev/uinput`, `/dev/uhid`,
and udev. Guidance SHALL name the exact commands and expected error symptoms.

#### Scenario: First-time pairing

- **WHEN** a user follows the documented pairing steps
- **THEN** the controller is paired and trusted before the service is started

#### Scenario: Wedged bond

- **WHEN** the controller is stuck in a half-paired state
- **THEN** the documentation provides a remove-and-repair recovery path

### Requirement: Validation status reporting

The project documentation SHALL distinguish hardware-free test results from
claims that require a physical controller, and SHALL record the date and target
environment for hardware validation.

#### Scenario: Hardware not yet available

- **WHEN** a feature has only synthetic or fake-transport coverage
- **THEN** the implementation status marks it as not hardware-validated

#### Scenario: Deck validation performed

- **WHEN** a feature is exercised on a Steam Deck
- **THEN** the implementation status records the environment, the observed
  behavior, and any remaining caveats
