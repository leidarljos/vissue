#!/usr/bin/env bash
# Print the `#` header that names where a bench output came from.
#
#     scripts/bench/bench_header.sh PROFILE [DIR]
#
# `PROFILE` is the cargo profile the measured binary was built in.
# `DIR` is the directory the bench writes to, for its filesystem; it defaults to `TMPDIR`.
# The host is named by its OS, architecture and CPU; the hostname is left out.
set -euo pipefail

profile=$1
dir=${2:-${TMPDIR:-/tmp}}

version() {
  if command -v "$1" > /dev/null; then
    "$1" --version 2>&1 | sed -n 1p
  else
    echo "not installed"
  fi
}

commit=$(git rev-parse HEAD)
git diff --quiet HEAD || commit="$commit with uncommitted changes"

case "$(uname -s)" in
  Darwin)
    os="macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
    cpu=$(sysctl -n machdep.cpu.brand_string)
    cores="$(sysctl -n hw.ncpu) logical, $(sysctl -n hw.physicalcpu) physical"
    governor="none: macOS has no cpufreq governor"
    mountpoint=$(df "$dir" | awk 'END { print $NF }')
    filesystem=$(mount | awk -v mp="$mountpoint" '$3 == mp { gsub(/[(,]/, "", $4); print $4 }')
    ;;
  *)
    os=$( (. /etc/os-release 2> /dev/null && echo "$PRETTY_NAME") || uname -s)
    cpu="$(awk -F': ' '/^model name/ { print $2; exit }' /proc/cpuinfo)"
    cpu="$cpu (family $(awk -F': ' '/^cpu family/ { print $2; exit }' /proc/cpuinfo), model $(awk -F': ' '/^model\t/ { print $2; exit }' /proc/cpuinfo))"
    cores="$(nproc) logical, $(lscpu | awk -F': *' '/^Thread\(s\) per core/ { print $2 }') thread(s) per core"
    governor=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2> /dev/null || echo "not exposed: no cpufreq in /sys")
    filesystem=$(findmnt -no FSTYPE -T "$dir")
    ;;
esac

echo "# commit: $commit"
echo "# date: $(date '+%Y-%m-%dT%H:%M:%S%z')"
echo "# os: $os, $(uname -m)"
echo "# kernel: $(uname -sr)"
echo "# cpu: $cpu"
echo "# cores: $cores"
echo "# governor: $governor"
echo "# filesystem: $filesystem"
echo "# build profile: $profile"
echo "# rustc: $(version rustc)"
echo "# git: $(version git)"
echo "# vim: $(version vim)"
echo "# emacs: $(version emacs)"
echo "# python3: $(version python3)"
