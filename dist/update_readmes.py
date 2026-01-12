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
    
    if not name_match:
        return None, None
        
    name = name_match.group(1)
    description = description_match.group(1).replace('\n', ' ').strip() if description_match else "A Tectonic sub-crate."
    
    # Handle multi-line descriptions that might have been matched greedily or weirdly
    if '"""' in description:
         # rudimentary fallback for multiline strings if regex failed to capture cleanly
         # simpler to just take the first sentence if it looks complex
         pass

    return name, description

def compute_published_name(name):
    if name == "tectonic":
        return "jxoesneon-tectonic"
    elif name.startswith("tectonic_"):
        return f'jxoesneon-{name.replace("tectonic_", "tectonic-")}'
    else:
        return f'jxoesneon-{name}'

def update_readme(crate_dir, name, description):
    readme_path = os.path.join(crate_dir, "README.md")
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

    # Remove existing header if it looks like one (title + badge)
    # We'll look for the first H1 and up to the first double newline after potential badges
    
    # Heuristic: Split into lines. Drop lines until we find one that doesn't look like:
    # - # Title
    # - [Badge]
    # - > Quote
    # - Empty line
    
    lines = existing_content.splitlines()
    remaining_lines = []
    header_passed = False
    
    # If we are creating a new file, simply use the content
    if not os.path.exists(readme_path):
         remaining_lines = [description, ""]
    else:
        # Simple parser to strip old header
        # We assume the old header is at the top. 
        # We skip lines until we hit a header-like structure standard in Tectonic
        
        # Actually, simpler approach: 
        # If the file starts with "# The `...` crate", skip that and the badge line.
        # Tectonic READMEs usually follow:
        # # The `name` crate
        # [badge]
        # Description
        
        idx = 0
        while idx < len(lines):
            line = lines[idx]
            stripped = line.strip()
            if stripped.startswith("# The `") and "` crate" in stripped:
                idx += 1
                continue
            if stripped.startswith("[![](http://meritbadge"):
                idx += 1
                continue
            if stripped.startswith("> [!NOTE]"): # Skip our own previous header if re-run
                 idx += 1
                 while idx < len(lines) and (lines[idx].strip().startswith(">") or lines[idx].strip() == ""):
                     idx += 1
                 continue
            
            # If we hit text or other things, stop skipping
            if stripped != "":
                break
            
            idx += 1
            
        remaining_lines = lines[idx:]

    new_content = header + "\n".join(remaining_lines)
    
    # URL Replacement
    new_content = new_content.replace("tectonic-typesetting/tectonic", "jxoesneon/tectonic")
    
    # Fix the specific "Main Git repository" link if it exists in a specific format
    # Often: - [Main Git repository](https://github.com/tectonic-typesetting/tectonic/).
    # The replace above catches most, but let's ensure the HTTPS URL is correct specifically
    new_content = new_content.replace("https://github.com/tectonic-typesetting/tectonic", "https://github.com/jxoesneon/tectonic")
    
    # Ensure a trailing newline
    if not new_content.endswith("\n"):
        new_content += "\n"

    with open(readme_path, "w") as f:
        f.write(new_content)
    print(f"Updated {readme_path}")

def main():
    crates_root = "crates"
    for item in os.listdir(crates_root):
        crate_dir = os.path.join(crates_root, item)
        if not os.path.isdir(crate_dir):
            continue
            
        toml_path = os.path.join(crate_dir, "Cargo.toml")
        if not os.path.exists(toml_path):
            continue
            
        name, description = get_crate_metadata(toml_path)
        if name:
            update_readme(crate_dir, name, description)

if __name__ == "__main__":
    main()
