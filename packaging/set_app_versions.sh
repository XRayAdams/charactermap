#!/bin/bash

echo "___________________________________________________________"
echo "Setting app version in all relevant files..."


# --- Configuration ---
CARGO_FILE="Cargo.toml"
DEBIAN_CONTROL_FILE="packaging/control"
DEBIAN_DESKTOP_FILE="packaging/gui/app.rayadams.charactermap.desktop"
SNAP_YAML_FILE="snap/snapcraft.yaml"
SNAP_DESKTOP_FILE="snap/gui/charactermap.desktop"
RPM_FILE="packaging/charactermap.spec"
MACHINE_ARCH=$(uname -m)
DEBIAN_CONTROL_FILE_ARCH="amd64"
AUR_CONTROL_FILE="packaging/PKGBUILD"

if [ "$MACHINE_ARCH" == "aarch64" ]; then
    MACHINE_ARCH="arm64"
    DEBIAN_CONTROL_FILE_ARCH="arm64"
    echo "Architecture was aarch64, updated to: $MACHINE_ARCH"
elif [ "$MACHINE_ARCH" == "x86_64" ]; then
    MACHINE_ARCH="x64"
    DEBIAN_CONTROL_FILE_ARCH="amd64"
fi
# ---------------------


# Check if files exist
if [ ! -f "$CARGO_FILE" ]; then
    echo "Error: File not found: $CARGO_FILE"
    exit 1
fi
if [ ! -f "$DEBIAN_CONTROL_FILE" ]; then
    echo "Error: File not found: $DEBIAN_CONTROL_FILE"
    exit 1
fi
if [ ! -f "$DEBIAN_DESKTOP_FILE" ]; then
    echo "Error: File not found: $DEBIAN_DESKTOP_FILE"
    exit 1
fi
if [ ! -f "$SNAP_YAML_FILE" ]; then
    echo "Error: File not found: $SNAP_YAML_FILE"
    exit 1
fi
if [ ! -f "$SNAP_DESKTOP_FILE" ]; then
    echo "Error: File not found: $SNAP_DESKTOP_FILE"
    exit 1
fi
if [ ! -f "$RPM_FILE" ]; then
    echo "Error: File not found: $RPM_FILE"
    exit 1
fi
if [ ! -f "$AUR_CONTROL_FILE" ]; then
    echo "Error: File not found: $AUR_CONTROL_FILE"
    exit 1
fi

# Read version from Cargo.toml (extracts the line with 'version =' and gets the value after the space)
APP_VERSION=$(grep -E '^\s*version = ' "$CARGO_FILE" | head -n1 | cut -d ' ' -f 3 | tr -d '"')

if [ -z "$APP_VERSION" ]; then
    echo "Error: Could not read version from $CARGO_FILE."
    exit 1
fi

echo "Version '$APP_VERSION' found in $CARGO_FILE"

# Parse version into APP_VERSION_SHORT and APP_BUILD
APP_VERSION_SHORT=$(echo "$APP_VERSION" | cut -d'+' -f1)
APP_BUILD=$(echo "$APP_VERSION" | cut -d'+' -f2)

# Use sed to find and replace the Version line in debian.yaml and desktop file
# This command looks for the line starting with '  Version:' and replaces the entire line.
sed -i "s/^\(\s*Version:\s*\).*\$/\1$APP_VERSION/" "$DEBIAN_CONTROL_FILE"
sed -i "s/^\(\s*Architecture:\s*\).*\$/\1$DEBIAN_CONTROL_FILE_ARCH/" "$DEBIAN_CONTROL_FILE"

sed -i "s/^\(\s*Version=\s*\).*\$/\1$APP_VERSION/" "$DEBIAN_DESKTOP_FILE"

sed -i "s/^\(\s*version:\s*\).*\$/\1$APP_VERSION/" "$SNAP_YAML_FILE"
sed -i "s/^\(\s*Version=\s*\).*\$/\1$APP_VERSION/" "$SNAP_DESKTOP_FILE"

# Update version in RPM spec file
sed -i "s/^\(\s*%define _version \s*\).*\$/\1$APP_VERSION_SHORT/" "$RPM_FILE"
sed -i "s/^\(\s*%define _release \s*\).*\$/\1$APP_BUILD/" "$RPM_FILE"

# Update version in AUR PKGBUILD
sed -i "s/^\(\s*pkgver=\).*\$/\1$APP_VERSION_SHORT/" "$AUR_CONTROL_FILE"
sed -i "s/^\(\s*pkgrel=\).*\$/\1$APP_BUILD/" "$AUR_CONTROL_FILE"

echo "Successfully updated version to $APP_VERSION in all relevant files."

