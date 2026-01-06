import os
import re
import sys

# Script to rename Tectonic crates for the jxoesneon fork in CI
# Usage: python3 rename_for_fork.py <new_version>

if len(sys.argv) < 2:
    print("Usage: python3 rename_for_fork.py <new_version>")
    sys.exit(1)

NEW_VERSION = sys.argv[1]

# We search for all Cargo.toml files in the workspace
TOML_FILES = ["Cargo.toml"]
for root, dirs, files in os.walk("crates"):
    if "Cargo.toml" in files:
        TOML_FILES.append(os.path.join(root, "Cargo.toml"))

def rename_and_update(toml_path):
    print(f"Processing {toml_path}...")
    with open(toml_path, "r") as f:
        content = f.read()

    # 1. Rename the package itself
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
    def rename_dep(match):
        orig_name = match.group(1)
        rest = match.group(2)
        if orig_name == "tectonic":
            new_name = "jxoesneon-tectonic"
        elif orig_name.startswith("tectonic_"):
            new_name = f'jxoesneon-{orig_name.replace("tectonic_", "tectonic-")}'
        else:
            new_name = f'jxoesneon-{orig_name}'

        # Use package alias so source code imports don't break
        if 'package =' not in rest:
            new_rest = f'package = "{new_name}", {rest}'
        else:
            new_rest = re.sub(r'package = "[^"]+"', f'package = "{new_name}"', rest)

        # Update version
        new_rest = re.sub(r"version = (['\"])[^'\"]+(['\"])", rf'version = \g<1>{NEW_VERSION}\g<2>', new_rest)
        return f'{orig_name} = {{ {new_rest} }}'

    content = re.sub(r'^(\w+)\s*=\s*{\s*(path\s*=\s*"[^"]+".*?)}', rename_dep, content, flags=re.MULTILINE)

    # 3. Features do NOT need renaming if we use aliases for dependencies

    # 4. Set version
    content = re.sub(r'^version = "[^"]+"', f'version = "{NEW_VERSION}"', content, flags=re.MULTILINE)

    with open(toml_path, "w") as f:
        f.write(content)

for toml in TOML_FILES:
    rename_and_update(toml)
