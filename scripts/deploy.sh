#!/bin/sh
set -e
host=$1
[ -n "$host" ] || { echo "usage: scripts/deploy.sh <windows-host>" >&2; exit 1; }
cd "$(dirname "$0")/.."
zip=$(mktemp -d)/src.zip
git ls-files -co --exclude-standard -z | xargs -0 zip -q "$zip"
home=C:/Users/$(ssh "$host" 'echo %USERNAME%' | tr -d '\r')
scp -q "$zip" "$host:$home/ezh.zip"
scp -q scripts/*.ps1 "$host:$home/"
ssh "$host" "powershell -NoProfile -ExecutionPolicy Bypass -File \"$home/remote.ps1\" ${INSTALLER:+-Installer} ${TEST:+-Test}"
