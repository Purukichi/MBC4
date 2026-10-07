#!/usr/bin/env bash
# Builds MBC4 on macOS (VST3 + AU + Standalone, universal arm64/x86_64).
# Needs: Xcode command line tools (xcode-select --install) and CMake (brew install cmake).
# JUCE is taken from ../Plugin/JUCE if present, otherwise downloaded automatically.
#
#   ./build_mac.sh            build only
#   ./build_mac.sh --install  build, then copy to ~/Library/Audio/Plug-Ins and sign ad-hoc
set -euo pipefail

cd "$(dirname "$0")"
BUILD=build-mac
CONFIG=Release

cmake -S . -B "$BUILD" -G Xcode
cmake --build "$BUILD" --config "$CONFIG" --parallel

ART="$BUILD/MBC4_artefacts/$CONFIG"
echo "VST3      : $ART/VST3/MBC4.vst3"
echo "AU        : $ART/AU/MBC4.component"
echo "Standalone: $ART/Standalone/MBC4.app"

# offline checks (crossover flatness, compression, CPU)
cmake --build "$BUILD" --config "$CONFIG" --target MBC4_Tests
"$BUILD/$CONFIG/MBC4_Tests" || true

if [[ "${1:-}" == "--install" ]]; then
    VST3_DIR="$HOME/Library/Audio/Plug-Ins/VST3"
    AU_DIR="$HOME/Library/Audio/Plug-Ins/Components"
    mkdir -p "$VST3_DIR" "$AU_DIR"
    rm -rf "$VST3_DIR/MBC4.vst3" "$AU_DIR/MBC4.component"
    cp -R "$ART/VST3/MBC4.vst3" "$VST3_DIR/"
    cp -R "$ART/AU/MBC4.component" "$AU_DIR/"

    # unsigned local build: ad-hoc sign so Apple Silicon hosts will load it
    codesign --force --deep --sign - "$VST3_DIR/MBC4.vst3" "$AU_DIR/MBC4.component"

    # make Logic / GarageBand rescan Audio Units
    killall -9 AudioComponentRegistrar 2>/dev/null || true
    echo "Installed to $VST3_DIR and $AU_DIR"
fi
