#!/bin/bash
set -e

# --- Configuration ---
MACHINE_ARCH=$(uname -m)
DEBIAN_ARCH="amd64"
APP_ID="app.rayadams.charactermap"
CARGO_FILE="Cargo.toml"

if [ "$MACHINE_ARCH" == "aarch64" ]; then
    MACHINE_ARCH="arm64"
    DEBIAN_ARCH="arm64"
elif [ "$MACHINE_ARCH" == "x86_64" ]; then
    MACHINE_ARCH="x64"
    DEBIAN_ARCH="amd64"
fi

# Clean and build app in release mode
cargo clean
cargo build --release

APP_NAME=$(grep -E '^\s*name = ' "$CARGO_FILE" | head -n1 | cut -d ' ' -f 3 | tr -d '"')
APP_VERSION_LONG=$(grep -E '^\s*version = ' "$CARGO_FILE" | head -n1 | cut -d ' ' -f 3 | tr -d '"')
APP_VERSION=$(echo "$APP_VERSION_LONG" | cut -d'+' -f1)
APP_BUILD=$(echo "$APP_VERSION_LONG" | cut -d'+' -f2)

strip target/release/"$APP_NAME"

# Set app versions to all files for packaging
packaging/set_app_versions.sh
rm -rf dist
mkdir -p dist

PACKAGE_DIR="$APP_NAME-$APP_VERSION+$APP_BUILD-$DEBIAN_ARCH"
printf "Creating DEB package in %s.deb\n" "$PACKAGE_DIR"
# Create the package directory
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR/usr/bin"
mkdir -p "$PACKAGE_DIR/usr/share/applications"
mkdir -p "$PACKAGE_DIR/usr/share/icons"
mkdir -p "$PACKAGE_DIR/usr/share/metainfo"
mkdir -p "$PACKAGE_DIR/usr/share/locale"

# Copy the built binary
install -m 755 "target/release/$APP_NAME" "$PACKAGE_DIR/usr/bin/$APP_NAME"
cp -r target/release/locale/. "$PACKAGE_DIR/usr/share/locale/"
cp packaging/gui/$APP_ID.desktop "$PACKAGE_DIR/usr/share/applications/"
cp packaging/gui/$APP_ID.png "$PACKAGE_DIR/usr/share/icons/"
cp packaging/$APP_ID.metainfo.xml "$PACKAGE_DIR/usr/share/metainfo/"

# Copy control file
mkdir -p "$PACKAGE_DIR/DEBIAN"
cp packaging/control "$PACKAGE_DIR/DEBIAN/control"

# Build the .deb package
dpkg-deb --build "$PACKAGE_DIR"

# Clean up
rm -rf "$PACKAGE_DIR"

cp "$PACKAGE_DIR.deb" dist/
rm "$PACKAGE_DIR.deb"

echo "DEB package created in dist/"
echo "___________________________________________________________"
# Package RPM
echo "Preparing RPM package"

# Create RPM build directories
RPM_BUILD_ROOT="$(pwd)/rpmbuild"
rm -rf "$RPM_BUILD_ROOT"

mkdir -p "$RPM_BUILD_ROOT/BUILD"
mkdir -p "$RPM_BUILD_ROOT/RPMS"
mkdir -p "$RPM_BUILD_ROOT/SOURCES"
mkdir -p "$RPM_BUILD_ROOT/SPECS"
mkdir -p "$RPM_BUILD_ROOT/SRPMS"

CHANGE_DATE=$(date +"%a %b %d %Y")
CHANGE_DATE="$CHANGE_DATE Konstantin Adamov (xrayadamo@gmail.com) - $APP_VERSION-$APP_BUILD"
sed "s/^*loghere$/* $CHANGE_DATE/" "packaging/$APP_NAME.spec" > "$RPM_BUILD_ROOT/SPECS/$APP_NAME.spec"

# Copy desktop and icon files, replacing Exec and TryExec with app name , by default it has full path for debian package
sed -e "s/Icon=$APP_ID/Icon=$APP_NAME/" -e "s/^\(Exec\|TryExec\)=.*$/\1=$APP_NAME/" "packaging/gui/$APP_ID.desktop"  > "$RPM_BUILD_ROOT/SOURCES/$APP_ID.desktop"
cp packaging/gui/"$APP_ID".png "$RPM_BUILD_ROOT/SOURCES/"
cp packaging/"$APP_ID".metainfo.xml "$RPM_BUILD_ROOT/SOURCES/"
cp LICENSE "$RPM_BUILD_ROOT/SOURCES/"
cp packaging/README.txt "$RPM_BUILD_ROOT/SOURCES/"

# Package the application files into a tarball
pushd target || exit
tar -czvf "$RPM_BUILD_ROOT/SOURCES/$APP_NAME-$APP_VERSION.tar.gz" "release/$APP_NAME" release/locale
popd || exit

# Build the RPM
rpmbuild -bb \
    --define "_topdir $RPM_BUILD_ROOT" \
    --define "_name $APP_NAME" \
    --define "_version $APP_VERSION" \
    --define "_release $APP_BUILD" \
    "$RPM_BUILD_ROOT/SPECS/$APP_NAME.spec"

# Move the RPM to the dist directory
find "$RPM_BUILD_ROOT/RPMS" -name "*.rpm" -exec mv {} dist/ \;

# Clean up
rm -rf "$RPM_BUILD_ROOT"
echo "RPM package created in dist/"

echo "___________________________________________________________"

#Packaging AUR
cp packaging/PKGBUILD .
PKGEXT='.pkg.tar.zst' COMPRESSZST=(zstd -c -T0 --auto-threads=logical -) env makepkg --nodeps -f
mv *.pkg.tar.zst dist/
rm -rf pkg/
rm PKGBUILD

echo "AUR package created in dist/"
echo "___________________________________________________________"

# Package TAR
echo "Preparing TAR archive"

ARCHIVE_NAME="${APP_NAME}-${APP_VERSION}+${APP_BUILD}-${MACHINE_ARCH}.tar.gz"
FULL_ARCHIVE_PATH="dist/${ARCHIVE_NAME}"
SOURCE_DIR="target/release"

# Only include the executable 
tar -czvf "$FULL_ARCHIVE_PATH" -C "$SOURCE_DIR" "$APP_NAME" > /dev/null
echo "TAR archive created in dist/"
echo "___________________________________________________________"
