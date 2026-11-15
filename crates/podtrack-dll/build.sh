#!/bin/sh
# Compile les DLL 64 et 32 bits et les place, sous leurs noms attendus par les jeux, dans
# podtrack-out/data/dll (fournies avec podtrack). Prérequis Fedora :
#   rust-std-static-x86_64-pc-windows-gnu rust-std-static-i686-pc-windows-gnu mingw64-gcc mingw32-gcc
set -eu
cd "$(dirname "$0")"
OUT=../podtrack-out/data/dll
mkdir -p "$OUT"
for t in x86_64-pc-windows-gnu i686-pc-windows-gnu; do
  linker=$( [ "$t" = x86_64-pc-windows-gnu ] && echo x86_64-w64-mingw32-gcc || echo i686-w64-mingw32-gcc )
  env "CARGO_TARGET_$(echo "$t" | tr a-z- A-Z_)_LINKER=$linker" cargo build --release --offline --target "$t" -q
done
cp target/x86_64-pc-windows-gnu/release/podtrack_dll.dll "$OUT/NPClient64.dll"
cp target/x86_64-pc-windows-gnu/release/podtrack_dll.dll "$OUT/freetrackclient64.dll"
cp target/i686-pc-windows-gnu/release/podtrack_dll.dll "$OUT/NPClient.dll"
cp target/i686-pc-windows-gnu/release/podtrack_dll.dll "$OUT/freetrackclient.dll"
ls -l "$OUT"
