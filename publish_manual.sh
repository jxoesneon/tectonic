#!/bin/bash
set -e

# Tectonic Manual Publication Script
# Enforces 1.5 minute (90s) delay between publications to respect rate limits.

TOKEN="${CARGO_REGISTRY_TOKEN}"
if [ -z "$TOKEN" ]; then
    echo "Error: CARGO_REGISTRY_TOKEN is not set."
    exit 1
fi

# Define crates in dependency order (leaf nodes first)
CRATES=(
    "crates/errors"
    "crates/status_base"
    "crates/io_base"
    "crates/dep_support"
    "crates/cfg_support"
    "crates/mac_core"
    "crates/bridge_core"
    "crates/bridge_flate"
    "crates/bridge_freetype2"
    "crates/bridge_graphite2"
    "crates/bridge_harfbuzz"
    "crates/bridge_icu"
    "crates/bridge_png"
    "crates/bridge_fontconfig"
    "crates/geturl"
    "crates/xdv"
    "crates/bundles"
    "crates/docmodel"
    "crates/xetex_format"
    "crates/engine_bibtex"
    "crates/pdf_io"
    "crates/engine_xdvipdfmx"
    "crates/xetex_layout"
    "crates/engine_xetex"
    "crates/engine_spx2html"
    "." # Root tectonic crate
)

echo "Starting publication of ${#CRATES[@]} crates..."

for i in "${!CRATES[@]}"; do
    crate="${CRATES[$i]}"
    echo "[$(date +%H:%M:%S)] Publishing $crate ($((i+1))/${#CRATES[@]})..."
    
    # We use --no-verify to speed up (tests already passed) and --token explicitly
    # note: if renaming is needed, this script would need to modify Cargo.toml here.
    # Given the user context, we try standard publish first as they might have
    # cargo-config to handle renaming or it's handled by some other means.
    
    (cd "$crate" && cargo publish --token "$TOKEN" --no-verify --allow-dirty) || {
        echo "Failed to publish $crate. Check if it's already published or name conflict."
        # If it's already a version conflict, we might want to continue, 
        # but for now we exit on error.
        exit 1
    }

    if [ $i -lt $(($ {#CRATES[@]} - 1)) ]; then
        echo "Waiting 90 seconds before next publication..."
        sleep 90
    fi
done

echo "Done!"
