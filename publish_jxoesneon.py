import os
import re
import time
import subprocess

# Tectonic Renaming and Publication Script for jxoesneon fork
# This script renames crates to the jxoesneon-tectonic-* prefix and publishes them.

PREFIX = "jxoesneon-"
NEW_VERSION = "0.16.2"
DELAY = 90  # seconds

CRATES_ORDER = [
    "crates/errors",
    "crates/status_base",
    "crates/io_base",
    "crates/dep_support",
    "crates/cfg_support",
    "crates/mac_core",
    "crates/bridge_core",
    "crates/bridge_flate",
    "crates/bridge_png",         # Must come before bridge_freetype2
    "crates/bridge_freetype2",
    "crates/bridge_graphite2",
    "crates/bridge_harfbuzz",
    "crates/bridge_icu",
    "crates/bridge_fontconfig",
    "crates/geturl",
    "crates/xdv",
    "crates/bundles",
    "crates/docmodel",
    "crates/xetex_format",
    "crates/engine_bibtex",
    "crates/pdf_io",
    "crates/engine_xdvipdfmx",
    "crates/xetex_layout",
    "crates/engine_xetex",
    "crates/engine_spx2html",
    "." # Root tectonic crate
]

def rename_and_update(path):
    print(f"Processing {path}...")
    toml_path = os.path.join(path, "Cargo.toml")
    with open(toml_path, "r") as f:
        content = f.read()

    # 1. Rename the package itself
    # [package]
    # name = "tectonic_..."
    def rename_pkg(match):
        name = match.group(1)
        if name == "tectonic":
            new_name = "jxoesneon-tectonic"
        elif name.startswith("tectonic_"):
            new_name = f'jxoesneon-{name.replace("tectonic_", "tectonic-")}'
        else:
            new_name = f'jxoesneon-{name}'
        return f'[package]\nname = "{new_name}"'

    content = re.sub(r'\[package\]\nname = "(tectonic(?:_[a-z0-9_]+)?)"', rename_pkg, content)

    # 2. Update dependencies on local tectonic crates
    # tectonic_errors = { path = "../errors", version = "..." }
    def rename_dep(match):
        dep_name = match.group(1)
        rest = match.group(2)
        if dep_name == "tectonic":
            new_name = "jxoesneon-tectonic"
        else:
            new_name = f'jxoesneon-{dep_name.replace("tectonic_", "tectonic-")}'
        # Also ensure we point to the NEW_VERSION, supporting both quote types
        new_rest = re.sub(r"version = (['\"])[^'\"]+(['\"])", f'version = \\g<1>{NEW_VERSION}\\g<2>', rest)
        return f'{new_name} = {{ {new_rest} }}'

    content = re.sub(r'^(tectonic(?:_[a-z0-9_]+)?)\s*=\s*{\s*(path\s*=\s*"[^"]+".*?)}', rename_dep, content, flags=re.MULTILINE)

    # 3. Update features that reference crates
    # serialization = ["serde", "tectonic_docmodel", "toml"]
    def rename_feature(match):
        val = match.group(0)
        # Find strings like "tectonic_docmodel" or "tectonic_docmodel/feat"
        # and replace with "jxoesneon-tectonic-docmodel"
        def repl_feat(m):
            name = m.group(1)
            suffix = m.group(2) or ""
            if name == "tectonic":
                return f'"jxoesneon-tectonic{suffix}"'
            if name.startswith("tectonic_"):
                return f'"jxoesneon-{name.replace("tectonic_", "tectonic-")}{suffix}"'
            return f'"jxoesneon-{name}{suffix}"'
        
        return re.sub(r'"(tectonic(?:_[a-z0-9_]+)?)(/[^"]+)?"', repl_feat, val)

    content = re.sub(r'^\[features\].*?(?=\n\[|\Z)', rename_feature, content, flags=re.MULTILINE | re.DOTALL)

    # 4. Set version
    content = re.sub(r'^version = "[^"]+"', f'version = "{NEW_VERSION}"', content, flags=re.MULTILINE)

    with open(toml_path, "w") as f:
        f.write(content)

def publish(path):
    token = os.environ.get("CARGO_REGISTRY_TOKEN")
    print(f"Publishing {path}...")
    try:
        subprocess.run(
            ["cargo", "publish", "--token", token, "--no-verify", "--allow-dirty"],
            cwd=path,
            check=True
        )
        return True
    except subprocess.CalledProcessError:
        print(f"FAILED to publish {path}")
        return False

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--start-from", type=int, default=0, help="Index to start publishing from (0-indexed)")
    args = parser.parse_args()
    
    # First, rename all
    for crate in CRATES_ORDER:
        rename_and_update(crate)
    
    # Then publish with delay, starting from specified index
    for i, crate in enumerate(CRATES_ORDER):
        if i < args.start_from:
            print(f"Skipping {crate} (already published)")
            continue
        if publish(crate):
            if i < len(CRATES_ORDER) - 1:
                print(f"Waiting {DELAY} seconds...")
                time.sleep(DELAY)
        else:
            print("Stopping due to error.")
            break
