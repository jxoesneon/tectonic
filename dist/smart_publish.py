import argparse
import subprocess
import time
import sys
import os
import urllib.request
import urllib.error
import json

# Rate Limitations from crates.io
# Publish: 1 req/min (refill), Burst 30.
# API: 1 req/sec.

BURST_LIMIT = 30
REFILL_RATE_SEC = 61 # 1 token per 61 seconds for safety
API_DELAY_SEC = 1.0 # 1 second between API calls

# Topological order of crates to publish
CRATES = [
    "crates/errors",
    "crates/status_base",
    "crates/io_base",
    "crates/dep_support",
    "crates/cfg_support",
    "crates/mac_core",
    "crates/bridge_core",
    "crates/bridge_flate",
    "crates/bridge_png",
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
    "."
]

class TokenBucket:
    def __init__(self, capacity, refill_rate_sec):
        self.capacity = capacity
        self.tokens = capacity
        self.last_refill = time.time()
        self.refill_rate_sec = refill_rate_sec

    def consume(self):
        self.refill()
        if self.tokens >= 1:
            self.tokens -= 1
            return True
        return False

    def refill(self):
        now = time.time()
        elapsed = now - self.last_refill
        # Refill 1 token every REFILL_RATE_SEC
        new_tokens = elapsed / self.refill_rate_sec
        
        if new_tokens > 0:
            self.tokens = min(self.capacity, self.tokens + new_tokens)
            self.last_refill = now
            
    def time_until_next_token(self):
        self.refill()
        if self.tokens >= 1:
            return 0
        
        # How much of a token do we need?
        needed = 1.0 - self.tokens
        return needed * self.refill_rate_sec

def get_pkg_info(crate_path):
    toml_path = os.path.join(crate_path, "Cargo.toml")
    name = None
    version = None
    
    with open(toml_path, "r") as f:
        for line in f:
            line = line.split('#')[0].strip()
            if line.startswith("name ="):
                name = line.split('=')[1].strip().strip('"')
            elif line.startswith("version ="):
                version = line.split('=')[1].strip().strip('"')
            
            if name and version:
                break
                
    return name, version

def check_published(name, version):
    url = f"https://crates.io/api/v1/crates/{name}/{version}"
    try:
        # User-Agent required by crates.io policy
        req = urllib.request.Request(
            url, 
            headers={'User-Agent': 'ferrotex-release-bot (jxoesneon/tectonic)'}
        )
        with urllib.request.urlopen(req) as response:
            data = json.loads(response.read().decode())
            if "version" in data and data["version"]["num"] == version:
                return True
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False
        print(f"  Warning: API check failed for {name} v{version}: {e}")
        return False
    except Exception as e:
        print(f"  Warning: API check failed for {name} v{version}: {e}")
        return False
        
    return False

def main():
    parser = argparse.ArgumentParser(description="Smart crate publisher with rate limiting")
    parser.add_argument("token", help="Cargo registry token")
    args = parser.parse_args()
    
    bucket = TokenBucket(BURST_LIMIT, REFILL_RATE_SEC)
    
    print(f"Starting Smart Publish. Burst: {BURST_LIMIT}, Refill: 1/{REFILL_RATE_SEC}s")
    
    for crate_path in CRATES:
        name, version = get_pkg_info(crate_path)
        if not name or not version:
            print(f"Error: Could not parse Cargo.toml for {crate_path}")
            sys.exit(1)
            
        print(f"[{name} v{version}] Checking status...")
        
        # Respect API Rate limit (1 req/sec)
        time.sleep(API_DELAY_SEC)
        
        if check_published(name, version):
            print(f"  Skipping: Already published.")
            continue
            
        # Needs publish. Check tokens.
        while not bucket.consume():
            wait_time = bucket.time_until_next_token()
            print(f"  Rate Limit Hit! Waiting {wait_time:.1f}s for token refill...")
            time.sleep(wait_time + 1) # +1 buffer
            
        print(f"  Publishing... (Tokens left: {bucket.tokens:.2f})")
        
        cmd = [
            "cargo", "publish", 
            "--token", args.token,
            "--no-verify", 
            "--allow-dirty",
            "--manifest-path", os.path.join(crate_path, "Cargo.toml")
        ]
        
        try:
             # Capture output to avoid noisy logs unless error
            subprocess.run(cmd, check=True, capture_output=True)
            print("  Success!")
        except subprocess.CalledProcessError as e:
            print(f"  Error publishing {name}:")
            print(e.stderr.decode())
            sys.exit(1)

if __name__ == "__main__":
    main()
