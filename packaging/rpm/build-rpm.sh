#!/bin/sh
# Construit le paquet source (SRPM) puis le RPM : archive de l'arbre de travail, archive des
# crates de Cargo.lock (seule étape qui télécharge), rpmbuild.
#   packaging/rpm/build-rpm.sh [--dll] [--srpm]
#     --dll   recompile d'abord les DLL Windows
#     --srpm  s'arrête au SRPM (pour mock ou COPR)
# Prérequis Fedora : rpm-build cargo-vendor-filterer, et pour le RPM gtk4-devel libadwaita-devel
# Résultat : target/rpm/SRPMS/podtrack-<version>-1.<dist>.src.rpm
#            target/rpm/RPMS/<arch>/podtrack-<version>-1.<dist>.<arch>.rpm
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
dll=0 etape=-ba
for a in "$@"; do
  case $a in
    --dll) dll=1 ;;
    --srpm) etape=-bs ;;
    *) echo "option inconnue : $a" >&2; exit 2 ;;
  esac
done
[ "$dll" = 1 ] && crates/podtrack-dll/build.sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
spec=packaging/rpm/podtrack.spec
if [ "$(sed -n 's/^Version: *//p' $spec)" != "$version" ]; then
  echo "$spec : Version différente de celle de Cargo.toml ($version)" >&2; exit 1
fi
[ -f Cargo.lock ] || { echo "Cargo.lock absent" >&2; exit 1; }
top="$root/target/rpm"
rm -rf "$top"
mkdir -p "$top/SOURCES"
# L'arbre de travail, et non git archive : le dépôt de construction est alimenté par rsync.
tar -czf "$top/SOURCES/podtrack-$version.tar.gz" \
  --exclude=./target --exclude=./.git --exclude=./vendor --exclude=./.cargo \
  --exclude=./crates/podtrack-dll/target \
  --transform "s|^\.|podtrack-$version|" .
# Crates des cibles Linux seulement.
vendor="$top/vendor"
mkdir -p "$vendor"
cargo vendor-filterer --all-features \
  --platform=x86_64-unknown-linux-gnu --platform=aarch64-unknown-linux-gnu \
  "$vendor/vendor" > "$top/vendor.log" 2>&1 || { tail -30 "$top/vendor.log" >&2; exit 1; }
tar -C "$vendor" -cJf "$top/SOURCES/podtrack-$version-vendor.tar.xz" vendor
rm -rf "$vendor"
rpmbuild $etape --define "_topdir $top" $spec > "$top/rpmbuild.log" 2>&1 \
  || { tail -30 "$top/rpmbuild.log" >&2; exit 1; }
find "$top/SRPMS" "$top/RPMS" -name '*.rpm' 2>/dev/null
