Name:           gearvr-evdev
Version:        %{?gearvr_version}%{!?gearvr_version:0.1.0}
Release:        1
Summary:        Samsung Gear VR Controller userspace driver
License:        MIT
URL:            https://github.com/kevindurb/gearvr-evdev
Source0:        %{name}-%{version}-x86_64-linux-gnu.tar.gz
BuildArch:      x86_64

# This package wraps a prebuilt release binary. Do not generate a debuginfo
# subpackage for it.
%global debug_package %{nil}

# Keep the spec usable outside the release script, where the version is passed
# with --define "gearvr_version <version>".
%{!?_userunitdir:%global _userunitdir %{_prefix}/lib/systemd/user}
%{!?_udevrulesdir:%global _udevrulesdir %{_prefix}/lib/udev/rules.d}

Requires:       dbus-libs
Requires:       systemd
Requires:       udev
Requires:       bluez

%description
A userspace driver for the Samsung Gear VR Controller (ET-YO324). It consumes
the controller's BlueZ GATT service and exposes it as a standard evdev gamepad,
with an optional motion node and an opt-in virtual DualSense backend.

This package installs the binary, the systemd user unit, and the udev rules.
It intentionally does not enable the user service. Enable it with
"systemctl --user enable --now gearvr.service", or globally in a bootc image
with "systemctl --global enable gearvr.service".

%prep
%autosetup -n %{name}-%{version}

%build
# The release tarball already contains the portable binary.

%install
install -d %{buildroot}%{_bindir}
install -m 0755 gearvr-cli %{buildroot}%{_bindir}/gearvr-cli

install -d %{buildroot}%{_userunitdir}
sed 's#%h/.local/bin/gearvr-cli#/usr/bin/gearvr-cli#' \
    systemd/gearvr.service > %{buildroot}%{_userunitdir}/gearvr.service

install -d %{buildroot}%{_udevrulesdir}
install -m 0644 udev/70-gearvr-evdev.rules %{buildroot}%{_udevrulesdir}/
install -m 0644 udev/60-gearvr-permissions.rules %{buildroot}%{_udevrulesdir}/

%post
# bootc image builds have no running udev; keep the scriptlet quiet there.
if [ -d /run/udev ]; then
    udevadm control --reload-rules >/dev/null 2>&1 || :
    udevadm trigger --subsystem-match=input >/dev/null 2>&1 || :
    udevadm trigger --subsystem-match=misc \
        --sysname-match=uinput --sysname-match=uhid >/dev/null 2>&1 || :
fi

%postun
if [ -d /run/udev ]; then
    udevadm control --reload-rules >/dev/null 2>&1 || :
fi

%files
%{_bindir}/gearvr-cli
%{_userunitdir}/gearvr.service
%{_udevrulesdir}/70-gearvr-evdev.rules
%{_udevrulesdir}/60-gearvr-permissions.rules
%license LICENSE
%doc README.md INSTALL.md
