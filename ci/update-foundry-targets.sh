#!/usr/bin/env bash
set -eo pipefail

# Discovers the latest foundry stable/RC tags and ensures
# ci/foundry-stable-pins.toml has entries for each.
# Run by CI nightly before the pin builds.

PINS_FILE="ci/foundry-stable-pins.toml"

echo "Fetching foundry tags..."
TAGS=$(curl -fsSL "https://api.github.com/repos/foundry-rs/foundry/tags?per_page=50" \
  | jq -r '.[].name')

# Latest patch per minor (e.g. v1.5.1, v1.4.4)
STABLES=$(echo "$TAGS" \
  | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
  | sort -Vr \
  | awk -F. '{minor=$1"."$2; if(!seen[minor]++){print}}' \
  | head -2)

# Latest RC
LATEST_RC=$(echo "$TAGS" \
  | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+-rc' \
  | sort -V \
  | tail -1)

echo "Stable targets: $STABLES"
echo "Latest RC: ${LATEST_RC:-none}"

# Read existing pins file to preserve manual [patches.*] sections
EXISTING_PATCHES=""
if [ -f "$PINS_FILE" ]; then
  EXISTING_PATCHES=$(python3 -c "
import re
with open('$PINS_FILE') as f: content = f.read()
# Extract all [patches.*] sections
for m in re.finditer(r'(\[patches\.[^\]]+\]\n(?:[^\[]*?))', content, re.DOTALL):
    print(m.group(1).rstrip())
    print()
" 2>/dev/null || echo "")
fi

# Write updated pins file
cat > "$PINS_FILE" << 'HEADER'
# Auto-updated by ci/update-foundry-targets.sh
# Manual [patches."<tag>"] sections are preserved across updates.
#
# [targets] lists the foundry versions to build against nightly.
# [patches."<tag>"] provides [patch.crates-io] overrides for versions
# that can't resolve deps cleanly.

HEADER

# Write [targets] section
{
  echo "[targets]"
  for tag in $STABLES; do
    echo "$tag = true"
  done
  if [ -n "$LATEST_RC" ]; then
    echo "$LATEST_RC = true"
  fi
  echo ""
} >> "$PINS_FILE"

# Restore existing patches
if [ -n "$EXISTING_PATCHES" ]; then
  echo "$EXISTING_PATCHES" >> "$PINS_FILE"
else
  # Seed with empty comment for manual additions
  cat >> "$PINS_FILE" << 'FOOTER'
# Add [patches."<tag>"] sections below for versions needing dep overrides.
# Example:
# [patches."v1.5.1"]
# svm-rs-builds = { git = "https://github.com/alloy-rs/svm-rs", tag = "v0.5.24" }
FOOTER
fi

echo "Updated $PINS_FILE:"
cat "$PINS_FILE"
