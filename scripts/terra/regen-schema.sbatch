#!/usr/bin/env bash
#SBATCH --job-name=vissue-schema
#SBATCH --time=00:20:00
#SBATCH --nodes=1
#SBATCH --ntasks-per-node=1
#SBATCH --cpus-per-task=4
#SBATCH --mem=4G
#SBATCH --partition=cpu
#SBATCH --output=%x-%j.out
# Regenerate crates/vissue-core/src/schema/vissue_capnp.rs from
# schema/vissue.capnp on the build host, where the capnpc-rust plugin can be
# built. Copy the result back and commit it beside the schema change.
set -euo pipefail
# capnp is a user install under ~/.local/bin; a batch shell does not carry it.
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
cd "$HOME/Git/Github/vissue"
command -v capnpc-rust >/dev/null || cargo install capnpc --locked 2>&1 | tail -1
capnp compile -o rust:crates/vissue-core/src/schema --src-prefix schema schema/vissue.capnp
head -5 crates/vissue-core/src/schema/vissue_capnp.rs
git status --short crates/vissue-core/src/schema
echo REGEN-DONE
