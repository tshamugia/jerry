#!/usr/bin/env bash
# Runs the core benchmarks (#44) of a base and a head checkout in turns, on one machine: base-1,
# head-1, base-2, head-2, ... each saved as a criterion baseline under $CRITERION_HOME. Then
# bench-compare.mjs compares them. Run it from the head checkout, with an empty $CRITERION_HOME.
#
# The base builds into its own target folder. Cargo names a workspace crate's artifacts by its
# path inside the workspace, so in a shared folder the base would quietly run the head's build.
#
#   BASE=<base checkout> CRITERION_HOME=<empty folder> [ROUNDS=3] [THRESHOLD=0.2]
#   [BASE_LABEL=...] [HEAD_LABEL=...] bash .github/scripts/bench.sh

set -euo pipefail

: "${BASE:?BASE must name the base checkout}"
: "${CRITERION_HOME:?CRITERION_HOME must name an empty folder for the results}"
export CRITERION_HOME
rounds=${ROUNDS:-3}
base_target="$BASE/target"

echo "::group::Build the head"
cargo bench -p jerry-core --no-run
echo "::endgroup::"

echo "::group::Build the base"
has_base=false
if [ ! -d "$BASE/crates/jerry-core/benches" ]; then
  echo "::notice title=No base benchmarks::The base has no benchmarks yet."
elif (cd "$BASE" && rustup toolchain install &&
  CARGO_TARGET_DIR="$base_target" cargo bench -p jerry-core --no-run); then
  has_base=true
else
  echo "::notice title=No base benchmarks::The base's benchmarks don't build."
fi
echo "::endgroup::"

for round in $(seq "$rounds"); do
  if [ "$has_base" = true ]; then
    echo "::group::Round $round: base"
    (cd "$BASE" && CARGO_TARGET_DIR="$base_target" \
      cargo bench -p jerry-core -- --save-baseline "base-$round" --noplot) ||
      echo "::warning title=Base benchmarks failed::Round $round of the base failed."
    echo "::endgroup::"
  fi
  echo "::group::Round $round: head"
  cargo bench -p jerry-core -- --save-baseline "head-$round" --noplot
  echo "::endgroup::"
done

node "$(dirname "$0")/bench-compare.mjs" "$CRITERION_HOME" --threshold "${THRESHOLD:-0.2}" \
  --base "${BASE_LABEL:-base}" --head "${HEAD_LABEL:-head}"
