%define _name charactermap
%define _version 1.1.4
%define _release 6
%define debug_package %{nil}

Name: %{_name}
Version: %{_version}
Release: %{_release}
Summary: Character Map
License: MIT
Group: Applications/Utilities
URL: https://github.com/XRayAdams/charactermap
BugURL: https://github.com/XRayAdams/charactermap/issues
Vendor: Konstantin Adamov

Source0: %{_name}-%{_version}.tar.gz
Source1: app.rayadams.charactermap.desktop
Source2: app.rayadams.charactermap.png
Source3: app.rayadams.charactermap.metainfo.xml

Requires: gtk4, libadwaita, libstdc++

%description
A character map and special-characters viewer for Linux, helping you browse installed fonts and quickly find any character you need.

%prep
%setup -q -n release

%build
# This section is intentionally left blank as we are packaging a pre-compiled Flutter application.

%install
rm -rf %{buildroot}

# Install binary
install -D -m 755 %{_name} %{buildroot}%{_bindir}/%{_name}

# Install locale files
find locale -name "*.mo" | while read mo; do \
    install -D -m 644 "$mo" %{buildroot}/usr/share/${mo}; \
done

# Copy the desktop file
install -D -m 644 %{SOURCE1} %{buildroot}/usr/share/applications/%{_name}.desktop

# Copy the application icon
install -D -m 644 %{SOURCE2} %{buildroot}/usr/share/icons/hicolor/512x512/apps/%{_name}.png

# Copy meta info
install -D -m 644 %{SOURCE3} %{buildroot}%{_datadir}/metainfo/%{name}.metainfo.xml
%files
%{_bindir}/%{_name}
/usr/share/locale/*/LC_MESSAGES/charactermap.mo
/usr/share/applications/%{_name}.desktop
/usr/share/icons/hicolor/512x512/apps/%{_name}.png
%{_datadir}/metainfo/%{name}.metainfo.xml

%changelog
*loghere
- Initial RPM release
