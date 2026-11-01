#!/usr/bin/env bash
#SBATCH --job-name=vissue-build
#SBATCH --time=00:20:00
#SBATCH --nodes=1
#SBATCH --ntasks-per-node=1
#SBATCH --cpus-per-task=4
#SBATCH --mem=8G
#SBATCH --partition=cpu
#SBATCH --output=%x-%j.out
# Release build of the tracker's binaries on the build host, copied to
# $HOME/stackbin where the seat's scripts and the laptop's copy step look.
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$HOME/Git/Github/vissue" || exit 1
git pull -q --ff-only 2>&1 | tail -1
echo "== vissue $(git log --oneline -1 | cut -c1-70)"
cargo build --release --locked -p vissue-cli -p vissue-mcp -p vissue-hud 2>&1 | tail -1
mkdir -p "$HOME/stackbin"
for b in vissue vissue-mcp vissue-hud; do
  cp -f "target/release/$b" "$HOME/stackbin/" && echo "copied $b $("$HOME/stackbin/$b" --version 2>/dev/null | head -1)"
done
echo BUILD-DONE
