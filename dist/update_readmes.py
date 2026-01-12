import os
import re
import sys

# Script to update README.md for Tectonic crates in the FerroTeX fork
# Usage: python3 dist/update_readmes.py

def get_crate_metadata(toml_path):
    with open(toml_path, "r") as f:
        content = f.read()
    
    name_match = re.search(r'\[package\]\nname = "(.*?)"', content)
    description_match = re.search(r'description = "(.*?)"', content, re.DOTALL)
    readme_match = re.search(r'readme = "(.*?)"', content)
    
    if not name_match:
        return None, None, None
        
    name = name_match.group(1)
    description = description_match.group(1).replace('\n', ' ').strip() if description_match else "A Tectonic sub-crate."
    readme_file = readme_match.group(1) if readme_match else "README.md"

    # Handle multi-line descriptions that might have been matched greedily or weirdly
    if '"""' in description:
         pass

    return name, description, readme_file

def compute_published_name(name):
    if name == "tectonic":
        return "jxoesneon-tectonic"
    elif name.startswith("tectonic_"):
        return f'jxoesneon-{name.replace("tectonic_", "tectonic-")}'
    else:
        return f'jxoesneon-{name}'

def update_readme(crate_dir, name, description, readme_filename):
    readme_path = os.path.join(crate_dir, readme_filename)
    published_name = compute_published_name(name)
    
    existing_content = ""
    if os.path.exists(readme_path):
        with open(readme_path, "r") as f:
            existing_content = f.read()
    else:
        existing_content = f"{description}\n"

    # Define the Standard Header
    header = f"""# The `{name}` crate

[![](http://meritbadge.herokuapp.com/{published_name})](https://crates.io/crates/{published_name})

> [!NOTE]
> This crate is part of the **FerroTeX** project, a specialized fork of Tectonic.
> It is published to crates.io as `{published_name}`.

"""

    lines = existing_content.splitlines()
    remaining_lines = []
    
    if not os.path.exists(readme_path):
         remaining_lines = [description, ""]
    else:
        idx = 0
        while idx < len(lines):
            line = lines[idx]
            stripped = line.strip()
            # Detect existing header variants
            if stripped.startswith("# The `") and "` crate" in stripped:
                idx += 1
                continue
            if "Tectonic (FerroTeX Fork)" in stripped: # Root readme header
                idx += 1
                continue
            if stripped.startswith("[![](http://meritbadge"):
                idx += 1
                continue
            if stripped.startswith("> [!NOTE]"):
                 idx += 1
                 while idx < len(lines) and (lines[idx].strip().startswith(">") or lines[idx].strip() == ""):
                     idx += 1
                 continue
            if stripped.startswith("> **Packages in this fork"):
                 idx += 1
                 while idx < len(lines) and (lines[idx].strip().startswith(">") or lines[idx].strip() == ""):
                     idx += 1
                 continue
            
            if stripped != "":
                break
            idx += 1
            
        remaining_lines = lines[idx:]

    new_content = header + "\n".join(remaining_lines)
    
    new_content = new_content.replace("tectonic-typesetting/tectonic", "jxoesneon/tectonic")
    new_content = new_content.replace("https://github.com/tectonic-typesetting/tectonic", "https://github.com/jxoesneon/tectonic")
    
    if not new_content.endswith("\n"):
        new_content += "\n"

    with open(readme_path, "w") as f:
        f.write(new_content)
    print(f"Updated {readme_path}")

def main():
    # Process root crate
    toml_path = "Cargo.toml"
    if os.path.exists(toml_path):
        name, description, readme_file = get_crate_metadata(toml_path)
        if name:
             # For root, we purposefully might want to update CARGO_README.md, 
             # but we should double check if that is what we want.
             # Yes, crates.io uses the file pointed to by 'readme'.
             update_readme(".", name, description, readme_file)

    # Process sub-crates
    crates_root = "crates"
    for item in os.listdir(crates_root):
        crate_dir = os.path.join(crates_root, item)
        if not os.path.isdir(crate_dir):
            continue
            
        toml_path = os.path.join(crate_dir, "Cargo.toml")
        if not os.path.exists(toml_path):
            continue
            
        name, description, readme_file = get_crate_metadata(toml_path)
        if name:
            update_readme(crate_dir, name, description, readme_file)

if __name__ == "__main__":
    main()
