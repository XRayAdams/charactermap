%define _name charactermap
%define _version 1.1.8
%define _release 20
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
Source4: LICENSE
Source5: README.txt

Requires: gtk4

%description
Browse installed fonts and find special characters with this
Linux character map viewer.

%prep
%setup -q -n release

%build
# This section is intentionally left blank as we are packaging a pre-compiled application.

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

# Copy license file
install -D -m 644 %{SOURCE4} %{buildroot}%{_datadir}/licenses/%{_name}/LICENSE

# Copy documentation
install -D -m 644 %{SOURCE5} %{buildroot}%{_datadir}/doc/%{_name}/README.txt

%find_lang %{_name}

%files -f %{_name}.lang
%{_bindir}/%{_name}
/usr/share/applications/%{_name}.desktop
/usr/share/icons/hicolor/512x512/apps/%{_name}.png
%{_datadir}/metainfo/%{name}.metainfo.xml
%license %{_datadir}/licenses/%{_name}/LICENSE
%doc %{_datadir}/doc/%{_name}/README.txt

%changelog
*loghere
- Initial RPM release
