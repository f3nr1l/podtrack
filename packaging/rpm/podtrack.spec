# Paquet binaire de podtrack, construit par build-rpm.sh à partir du binaire compilé.
Name:           podtrack
Version:        %{pt_version}
Release:        1%{?dist}
Summary:        Suivi de tête par AirPods (jeux TrackIR/Proton, opentrack, pointage par la tête)
License:        Apache-2.0
URL:            https://github.com/f3nr1l/podtrack
Source0:        %{name}-%{version}.tar.gz

Requires:       gtk4
Requires:       libadwaita
Requires:       bluez
Requires:       librsvg2
Requires:       systemd

%global debug_package %{nil}

%description
podtrack utilise les capteurs de mouvement des AirPods comme traqueur de tête sous Linux :
DLL TrackIR et FreeTrack pour les jeux sous Wine / Proton (tables des jeux « Enhanced »
comprises), sortie opentrack, manette virtuelle et pointage par la tête. Interface GNOME
(GTK 4, libadwaita), icône de barre, service utilisateur systemd.

%prep
%setup -q

%install
install -Dm755 podtrack %{buildroot}%{_bindir}/podtrack
install -Dm644 podtrack.service %{buildroot}%{_userunitdir}/podtrack.service
install -Dm644 io.github.f3nr1l.PodTrack.desktop %{buildroot}%{_datadir}/applications/io.github.f3nr1l.PodTrack.desktop
install -Dm644 podtrack.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/podtrack.svg

%post
%systemd_user_post podtrack.service

%preun
%systemd_user_preun podtrack.service

%files
%license LICENSE NOTICE
%doc README.md
%{_bindir}/podtrack
%{_userunitdir}/podtrack.service
%{_datadir}/applications/io.github.f3nr1l.PodTrack.desktop
%{_datadir}/icons/hicolor/scalable/apps/podtrack.svg

%changelog
* Sun Nov 15 2026 René Onier <f3nr1l@icloud.com> - %{pt_version}-1
- Première version : portage Rust de MousePods, interface GNOME, DLL en Rust.
