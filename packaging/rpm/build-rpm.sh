#!/bin/sh
# Construit le RPM : compile podtrack (et les DLL si demandé), prépare l'archive, lance rpmbuild.
#   packaging/rpm/build-rpm.sh [--dll]
# Résultat : target/rpm/RPMS/x86_64/podtrack-<version>-1.<dist>.x86_64.rpm
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
[ "${1:-}" = "--dll" ] && crates/podtrack-dll/build.sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release --offline -q -p podtrack
top="$root/target/rpm"
stage="$top/stage/podtrack-$version"
rm -rf "$top"
mkdir -p "$stage" "$top/SOURCES"
cp target/release/podtrack README.md LICENSE NOTICE "$stage/"
cp packaging/io.github.f3nr1l.PodTrack.desktop "$stage/"
cp crates/podtrack/data/icons/podtrack.svg "$stage/"
# le paquet installe le binaire dans /usr/bin
sed 's|%h/.local/bin/podtrack|/usr/bin/podtrack|' packaging/podtrack.service > "$stage/podtrack.service"
tar -C "$top/stage" -czf "$top/SOURCES/podtrack-$version.tar.gz" "podtrack-$version"
rpmbuild -bb --define "_topdir $top" --define "pt_version $version" packaging/rpm/podtrack.spec >/dev/null
find "$top/RPMS" -name '*.rpm'
