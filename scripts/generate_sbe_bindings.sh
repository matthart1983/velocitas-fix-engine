#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SBE_TOOL_JAR="${SBE_TOOL_JAR:-}"

if [[ -z "$SBE_TOOL_JAR" ]]; then
  echo "Set SBE_TOOL_JAR to the official aeron-io/simple-binary-encoding sbe-all jar." >&2
  echo "Example: SBE_TOOL_JAR=/path/to/sbe-all-1.38.0-SNAPSHOT.jar $0" >&2
  exit 1
fi

java --add-opens java.base/jdk.internal.misc=ALL-UNNAMED \
  -Dsbe.output.dir="$ROOT_DIR/generated/rust" \
  -Dsbe.target.language=Rust \
  -Dsbe.target.namespace=velocitas_fix_sbe \
  -jar "$SBE_TOOL_JAR" \
  "$ROOT_DIR/schema/fix_aeron_envelope.xml"
