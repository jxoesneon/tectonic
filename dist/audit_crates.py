import os
import re
import json
import urllib.request
import urllib.error

# Audit script to check published versions of FerroTeX crates
# standardizes on finding jxoesneon- prefixed crates

def get_crate_names():
    crates = []
    # Root crate
    crates.append(("tectonic", "jxoesneon-tectonic"))
    
    # Sub crates
    crates_root = "crates"
    if os.path.exists(crates_root):
        for item in os.listdir(crates_root):
            if not os.path.isdir(os.path.join(crates_root, item)):
                continue
            toml_path = os.path.join(crates_root, item, "Cargo.toml")
            if os.path.exists(toml_path):
                with open(toml_path, "r") as f:
                    content = f.read()
                match = re.search(r'\[package\]\nname = "(.*?)"', content)
                if match:
                    orig_name = match.group(1)
                    if orig_name == "tectonic":
                        pub_name = "jxoesneon-tectonic"
                    elif orig_name.startswith("tectonic_"):
                        pub_name = f'jxoesneon-{orig_name.replace("tectonic_", "tectonic-")}'
                    else:
                        pub_name = f'jxoesneon-{orig_name}'
                    crates.append((orig_name, pub_name))
    return crates

def fetch_versions(pub_name):
    url = f"https://crates.io/api/v1/crates/{pub_name}"
    try:
        req = urllib.request.Request(
            url, 
            headers={'User-Agent': 'ferrotex-audit (jxoesneon/tectonic)'}
        )
        with urllib.request.urlopen(req) as response:
            data = json.loads(response.read().decode())
            versions = [v["num"] for v in data.get("versions", [])]
            return versions
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None # Not published yet
        return f"Error: {e}"
    except Exception as e:
        return f"Error: {e}"

def main():
    print(f"{'Original Name':<30} | {'Published Name':<40} | {'Versions'}")
    print("-" * 100)
    
    crates = get_crate_names()
    crates.sort()
    
    for orig, pub in crates:
        versions = fetch_versions(pub)
        if versions is None:
            ver_str = "(Not Published)"
        elif isinstance(versions, str):
            ver_str = versions
        else:
            ver_str = ", ".join(versions)
            
        print(f"{orig:<30} | {pub:<40} | {ver_str}")

if __name__ == "__main__":
    main()
