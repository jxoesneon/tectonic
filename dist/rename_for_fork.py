import os
import glob
import re
import sys

# Script to rename Tectonic crates for the jxoesneon fork in CI
# Usage: python3 rename_for_fork.py <new_version>

if len(sys.argv) < 2:
    print("Usage: python3 rename_for_fork.py <new_version>")
    sys.exit(1)

NEW_VERSION = sys.argv[1]

# We search for all Cargo.toml files in the workspace
TOML_FILES = ["Cargo.toml"] + glob.glob("crates/*/Cargo.toml")

def process_line(line, in_package_section):
    # 1. Rename package name in [package]
    if in_package_section:
        # Debug logging to verify what CI sees
        print(f"    [package] line: {repr(line)}")

        m_name = re.match(r'^\s*name\s*=\s*"(tectonic(?:_[a-z0-9_]+)?)"', line)
        if m_name:
            name = m_name.group(1)
            if name == "tectonic":
                new_name = "jxoesneon-tectonic"
            elif name.startswith("tectonic_"):
                new_name = f'jxoesneon-{name.replace("tectonic_", "tectonic-")}'
            else:
                new_name = f'jxoesneon-{name}'
            print(f"  Rewriting name: {name} -> {new_name}")
            return f'name = "{new_name}"\n'
    
        # 2. Update version in [package]
        # Regex matches 'version' key with optional whitespace and equals sign
        if re.match(r'^\s*version\s*=', line):
            print(f"  Rewriting version: {line.strip()} -> {NEW_VERSION}")
            return f'version = "{NEW_VERSION}"\n'

    # 3. Rename dependencies (anywhere, usually [dependencies])
    # path = "..." implies internal dep
    if "path =" in line and "tectonic" in line:
        # Match dependency definition
        # dep = { path = "...", version = "..." }
        # or dep = { version = "...", path = "..." }
        
        # Regex to capture the dependency name and the content inside braces
        # Note: This handles single-line deps. Multi-line deps are harder but Tectonic uses single-line for internal deps.
        m = re.match(r'^\s*(\w+)\s*=\s*{(.*)}', line)
        if m:
            orig_name = m.group(1)
            rest = m.group(2)
            
            if "path" in rest and orig_name.startswith("tectonic"):
                # Calculate new package name
                if orig_name == "tectonic":
                    new_pkg = "jxoesneon-tectonic"
                elif orig_name.startswith("tectonic_"):
                    new_pkg = f'jxoesneon-{orig_name.replace("tectonic_", "tectonic-")}'
                else:
                    new_pkg = f'jxoesneon-{orig_name}'
                
                # Replace/Insert package key
                if 'package =' not in rest:
                    # Insert at start of braces
                    rest = f' package = "{new_pkg}",{rest}'
                else:
                    # Replace existing
                    rest = re.sub(r'package\s*=\s*"[^"]+"', f'package = "{new_pkg}"', rest)
                
                # Update version
                # Only if version key exists
                if 'version =' in rest:
                    rest = re.sub(r'version\s*=\s*([\'"])[^"\']+\1', f'version = "{NEW_VERSION}"', rest)
                
                return f'{orig_name} = {{{rest}}}\n'

    return line

def rename_and_update(toml_path):
    print(f"Processing {toml_path}...")
    with open(toml_path, "r") as f:
        lines = f.readlines()

    new_lines = []
    in_package_section = False
    
    for line in lines:
        stripped = line.strip()
        
        # Detect sections
        if stripped.startswith("[") and stripped.endswith("]"):
            if stripped == "[package]":
                in_package_section = True
            else:
                in_package_section = False
        
        new_line = process_line(line, in_package_section)
        new_lines.append(new_line)

    with open(toml_path, "w") as f:
        f.writelines(new_lines)

for toml in TOML_FILES:
    rename_and_update(toml)
