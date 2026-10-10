#!/usr/bin/env bash
# Regenerate the 9 captures (raw ANSI + plain-stripped) from the fixture.
set -u
cd "$(dirname "$0")/.."
cargo build -q -p example-doctor
BIN=../target/debug/example-doctor
for face in plain rich compact; do
  for theme in mono clinical contrast; do
    $BIN scan ./fixture --face $face --theme $theme --color always > samples/$face-$theme.txt
    sed -E $'s/\x1b\\[[0-9;]*m//g' samples/$face-$theme.txt > samples/$face-$theme.plain.txt
  done
done
