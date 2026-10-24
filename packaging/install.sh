#!/bin/sh
#
# Install the Gear VR driver from an unpacked release archive.
#
# Modes:
#   ./install.sh --user        install the binary and user service (no root)
#   sudo ./install.sh --udev   install the device-permission rules
#   ./install.sh --uninstall   remove the user binary, unit, and enablement
#
# The user mode deliberately refuses to run as root; udev mode requires root.
# Captures and calibration profiles are never removed by this script.

set -eu

PROG=gearvr-cli
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ASSET_DIR=${GEARVR_ASSET_DIR:-$SCRIPT_DIR}
BIN_DIR=${GEARVR_BIN_DIR:-"$HOME/.local/bin"}
UNIT_DIR=${GEARVR_UNIT_DIR:-${XDG_CONFIG_HOME:-"$HOME/.config"}/systemd/user}
UDEV_DIR=${GEARVR_UDEV_DIR:-/etc/udev/rules.d}

usage() {
    cat <<EOF
Usage: $0 [--user|--udev|--uninstall]

  --user       Install $PROG to $BIN_DIR and gearvr.service to
               $UNIT_DIR, then enable and start the user service.
  --udev       Install the udev rules to $UDEV_DIR. Must be run as root.
  --uninstall  Disable and remove the user binary, unit, and enablement.
               Captures and calibration files are preserved.

Environment overrides (mainly for testing):
  GEARVR_ASSET_DIR  directory containing gearvr-cli, systemd/, and udev/
  GEARVR_BIN_DIR    user binary directory
  GEARVR_UNIT_DIR   user unit directory
  GEARVR_UDEV_DIR   udev rules directory
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

has_user_manager() {
    command -v systemctl >/dev/null 2>&1 \
        && systemctl --user show-environment >/dev/null 2>&1
}

install_user() {
    [ "$(id -u)" -ne 0 ] || die "run --user as the user who will use the driver, not as root"
    [ -f "$ASSET_DIR/$PROG" ] || die "missing $ASSET_DIR/$PROG; run this from an unpacked release archive"
    [ -f "$ASSET_DIR/systemd/gearvr.service" ] || die "missing $ASSET_DIR/systemd/gearvr.service"

    install -d "$BIN_DIR" "$UNIT_DIR"
    install -m 0755 "$ASSET_DIR/$PROG" "$BIN_DIR/$PROG"
    install -m 0644 "$ASSET_DIR/systemd/gearvr.service" "$UNIT_DIR/gearvr.service"

    echo "Installed $BIN_DIR/$PROG"
    echo "Installed $UNIT_DIR/gearvr.service"

    if has_user_manager; then
        systemctl --user daemon-reload
        systemctl --user enable --now gearvr.service
        echo "Enabled and started gearvr.service"
    else
        echo
        echo "No reachable user systemd manager. After logging in, run:"
        echo "  systemctl --user daemon-reload"
        echo "  systemctl --user enable --now gearvr.service"
    fi

    echo
    echo "The driver needs access to /dev/uinput and /dev/uhid."
    echo "Install the udev rules as root:"
    echo "  sudo \"$SCRIPT_DIR/install.sh\" --udev"
}

install_udev() {
    [ "$(id -u)" -eq 0 ] || die "run --udev as root (for example: sudo \"$SCRIPT_DIR/install.sh\" --udev)"
    [ -d "$ASSET_DIR/udev" ] || die "missing $ASSET_DIR/udev; run this from an unpacked release archive"

    install -d "$UDEV_DIR"
    install -m 0644 "$ASSET_DIR"/udev/*.rules "$UDEV_DIR"/
    echo "Installed udev rules in $UDEV_DIR"

    if command -v udevadm >/dev/null 2>&1; then
        udevadm control --reload-rules || true
        udevadm trigger --subsystem-match=input || true
        udevadm trigger --subsystem-match=misc \
            --sysname-match=uinput --sysname-match=uhid || true
    fi

    echo "Device permissions refreshed. Reconnect the controller if it is already running."
}

uninstall_user() {
    [ "$(id -u)" -ne 0 ] || die "run --uninstall as the user who installed the driver, not as root"

    if has_user_manager; then
        systemctl --user disable --now gearvr.service >/dev/null 2>&1 || true
    fi

    rm -f "$UNIT_DIR/gearvr.service" "$BIN_DIR/$PROG"

    if has_user_manager; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi

    echo "Removed $BIN_DIR/$PROG and $UNIT_DIR/gearvr.service"
    echo "Captures and calibration profiles were left in place."
    echo
    echo "To remove the udev rules as root:"
    echo "  sudo rm -f $UDEV_DIR/60-gearvr-permissions.rules $UDEV_DIR/70-gearvr-evdev.rules"
    echo "  sudo udevadm control --reload-rules"
}

MODE=
for arg in "$@"; do
    case "$arg" in
        --user|--udev|--uninstall)
            MODE=$arg
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $arg" >&2
            usage >&2
            exit 2
            ;;
    esac
done
[ -n "$MODE" ] || MODE=--user

case "$MODE" in
    --user) install_user ;;
    --udev) install_udev ;;
    --uninstall) uninstall_user ;;
esac
