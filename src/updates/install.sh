#!/bin/sh
set -eu
pid=$1
stage=$2
target=$3
backup=$4
platform=$5
work=$6
# Wait for Ferrum to save its vault and close all sessions normally.
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
mv "$target" "$backup"
if ! mv "$stage" "$target"; then
    mv "$backup" "$target"
    exit 1
fi
if [ "$platform" = macos ]; then
    if ! /usr/bin/open -n "$target"; then
        mv "$target" "$stage"
        mv "$backup" "$target"
        /usr/bin/open -n "$target"
        exit 1
    fi
else
    "$target" </dev/null >/dev/null 2>&1 &
fi
# Retain the old build beside the installation as a recovery copy.
rm -rf "$work"
