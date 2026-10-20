#!/usr/bin/env bash
# Builds the minimal FFmpeg the app links for hardware H.264 encoding (ADR 0058): only
# libavcodec + libavutil with the platform's hardware encoder, static, LGPL (no GPL parts).
#
#   tools/ffmpeg/build.sh <rust target> <output dir>
#
# Targets: aarch64-apple-darwin, x86_64-apple-darwin (VideoToolbox), x86_64-pc-windows-msvc
# (Media Foundation + swscale; run from an MSYS2 shell that inherits the MSVC environment,
# with make, nasm and patch). The output dir gets include/ and lib/; point FFMPEG_DIR at it
# when building the app (crates/codec/build.rs).
set -euo pipefail

VERSION=7.1.2
SHA256=089bc60fb59d6aecc5d994ff530fd0dcb3ee39aa55867849a2bbc4e555f9c304

target=${1:?rust target}
out=${2:?output dir}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/ffmpeg.tar.xz" "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz"
if command -v sha256sum >/dev/null; then
  echo "$SHA256  $work/ffmpeg.tar.xz" | sha256sum -c -
else
  echo "$SHA256  $work/ffmpeg.tar.xz" | shasum -a 256 -c -
fi
tar -xf "$work/ffmpeg.tar.xz" -C "$work"
src="$work/ffmpeg-$VERSION"
for patch in "$here"/patches/*.patch; do
  patch -d "$src" -p1 < "$patch"
done

common=(
  --prefix="$out"
  --enable-static --disable-shared --enable-pic
  --disable-programs --disable-doc --disable-network --disable-autodetect
  --disable-everything
  --disable-avdevice --disable-avformat --disable-avfilter
  --disable-swscale --disable-swresample --disable-postproc
)

case "$target" in
  aarch64-apple-darwin | x86_64-apple-darwin)
    arch=${target%%-*}
    clang_arch=$([ "$arch" = aarch64 ] && echo arm64 || echo x86_64)
    flags="-arch $clang_arch -mmacosx-version-min=11.0"
    # The SDK of the selected Xcode, not whatever the plain clang shim would pick.
    export SDKROOT=${SDKROOT:-$(xcrun --sdk macosx --show-sdk-path)}
    extra=()
    # Cross builds (e.g. Intel on an Apple Silicon runner) need no assembler: the hardware
    # does the encoding.
    if [ "$(uname -m)" != "$clang_arch" ]; then
      extra=(--enable-cross-compile --target-os=darwin --disable-asm)
    fi
    (cd "$src" && ./configure "${common[@]}" ${extra[@]+"${extra[@]}"} \
      --arch="$arch" --cc=clang \
      --extra-cflags="$flags" --extra-ldflags="$flags" \
      --enable-videotoolbox --enable-encoder=h264_videotoolbox)
    ;;
  x86_64-pc-windows-msvc)
    (cd "$src" && ./configure "${common[@]}" \
      --toolchain=msvc --target-os=win64 --arch=x86_64 --extra-cflags=-MD \
      --enable-swscale --enable-mediafoundation --enable-encoder=h264_mf)
    ;;
  *)
    echo "unsupported target: $target" >&2
    exit 1
    ;;
esac

make -C "$src" -j"$(sysctl -n hw.ncpu 2>/dev/null || nproc)"
make -C "$src" install
# MSVC links "name.lib"; FFmpeg names its static libraries "libname.a".
if [ "$target" = x86_64-pc-windows-msvc ]; then
  for lib in "$out"/lib/lib*.a; do
    name=$(basename "$lib" .a)
    cp "$lib" "$out/lib/${name#lib}.lib"
  done
fi
echo "FFmpeg $VERSION for $target in $out"
