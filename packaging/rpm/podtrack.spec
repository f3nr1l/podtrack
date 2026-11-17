# Paquet source de podtrack : l'application est compilée par rpmbuild, sans réseau, avec les
# crates de Cargo.lock copiées dans une seconde archive (build-rpm.sh). C'est ce que demandent
# mock et COPR. La version doit être celle de Cargo.toml : build-rpm.sh le vérifie.
Name:           podtrack
Version:        0.1.0
Release:        1%{?dist}
Summary:        Suivi de tête par AirPods (jeux TrackIR/Proton, opentrack, pointage par la tête)
# Code sous Apache-2.0 ; crates liées au binaire sous leurs licences (relevé du 2026-11-17 :
# cargo tree -p podtrack -e normal --format '{l}').
License:        Apache-2.0 AND (MIT OR Apache-2.0) AND MIT AND (Unlicense OR MIT) AND Unlicense AND MPL-2.0 AND Unicode-3.0 AND (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT)
URL:            https://github.com/f3nr1l/podtrack
Source0:        %{name}-%{version}.tar.gz
# Crates de Cargo.lock, cibles Linux seulement (cargo vendor-filterer).
Source1:        %{name}-%{version}-vendor.tar.xz

# Le binaire embarque les DLL Windows déjà compilées (crates/podtrack-out/data/dll).
ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  gtk4-devel
BuildRequires:  libadwaita-devel
BuildRequires:  systemd-rpm-macros
BuildRequires:  desktop-file-utils

# GTK et libadwaita : trouvées par rpm (bibliothèques liées).
Requires:       bluez
Requires:       librsvg2

# Pas de symboles de mise au point dans le profil release : pas de paquet debuginfo.
%global debug_package %{nil}

%description
podtrack utilise les capteurs de mouvement des AirPods comme traqueur de tête sous Linux :
DLL TrackIR et FreeTrack pour les jeux sous Wine / Proton (tables des jeux « Enhanced »
comprises), sortie opentrack, manette virtuelle et pointage par la tête. Interface GNOME
(GTK 4, libadwaita), icône de barre, service utilisateur systemd.

%prep
%autosetup -a1
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
export CARGO_BUILD_JOBS=%{_smp_build_ncpus}
cargo build --release --offline --locked -p podtrack

%install
install -Dm755 target/release/podtrack %{buildroot}%{_bindir}/podtrack
# le paquet installe le binaire dans /usr/bin
sed 's|%%h/.local/bin/podtrack|%{_bindir}/podtrack|' packaging/podtrack.service > podtrack.service
install -Dm644 podtrack.service %{buildroot}%{_userunitdir}/podtrack.service
install -Dm644 packaging/io.github.f3nr1l.PodTrack.desktop %{buildroot}%{_datadir}/applications/io.github.f3nr1l.PodTrack.desktop
install -Dm644 crates/podtrack/data/icons/podtrack.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.f3nr1l.PodTrack.svg
install -Dm644 packaging/io.github.f3nr1l.PodTrack.Daemon.service %{buildroot}%{_datadir}/dbus-1/services/io.github.f3nr1l.PodTrack.Daemon.service

%check
export CARGO_BUILD_JOBS=%{_smp_build_ncpus}
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.f3nr1l.PodTrack.desktop
grep -q '^ExecStart=%{_bindir}/podtrack ' %{buildroot}%{_userunitdir}/podtrack.service
cargo test --release --offline --locked --workspace

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
%{_datadir}/icons/hicolor/scalable/apps/io.github.f3nr1l.PodTrack.svg
%{_datadir}/dbus-1/services/io.github.f3nr1l.PodTrack.Daemon.service

%changelog
* Tue Nov 17 2026 René Onier <f3nr1l@icloud.com> - 0.1.0-1
- Paquet source : compilation par rpmbuild avec les crates fournies, pour mock et COPR.
- Première version : portage Rust de MousePods, interface GNOME, DLL en Rust.
