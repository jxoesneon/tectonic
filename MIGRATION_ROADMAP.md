# Tectonic Pure-Rust Engine Migration Roadmap

> **Status**: Ratified by the Ciel Council of Five (Verdict: `PASS_CONDITIONAL`, Score: `8.125 / 10.0`)  
> **Governance Docket**: `~/.ciel/council/council-20260918-tectonic-migration-roadmap.json`  
> **Target End-State**: 100% Pure-Rust, memory-safe, thread-safe, re-entrant TeX/LaTeX typesetting engine with zero C/C++ compiler dependencies, producing 1:1 bitwise/visually identical output across the entire CTAN and TeXLive corpus.

---

## 1. Executive Summary & Core Axioms

Tectonic's mission is to modernize TeX without sacrificing the four decades of historical quirks that make LaTeX, BibTeX, TikZ, and standard packages function correctly. The ultimate goal of this fork is the **complete migration of the legacy Web2C/XeTeX C/C++ engine into pure, safe Rust**.

### Core Axioms
1. **Never Break Legacy Documents**: TeX is an execution automaton, not an abstract syntax tree. LaTeX packages (`expl3`, `tikz`, `amsmath`, `hyperref`) intentionally exploit Knuth's token scanning, dynamic catcode changes, and fixed-point math quirks. Parity is non-negotiable.
2. **Deterministic Fixed-Point Layout**: All layout and dimension arithmetic must preserve 32-bit fixed-point (`scaled_t`: $1\,\text{pt} = 65,536\,\text{sp}$) with exact two's-complement integer semantics. IEEE 754 floating-point arithmetic is strictly prohibited in TeX layout.
3. **Decouple the Backend First (Strangler Fig Strategy)**: The Extended DVI (`.xdv`) byte format emitted by the TeX engine is a frozen, declarative opcode stream. Porting the backend first eliminates >50% of legacy C without risking TeX stomach layout divergence.
4. **Zero Global Static State**: Modern execution requires multi-tenant concurrency and instant Clone-on-Write (CoW) state branching. The pure-Rust engine encapsulates all state inside an instantiable `TeXEngineState` struct with zero process-global `static mut` variables.
5. **Pure WASM Portability**: The engine must compile natively to `wasm32-unknown-unknown` with zero POSIX or Emscripten/Asyncify shims, enabling <8MB client-side browser compilation and edge execution.

---

## 2. Current Architecture Inventory

As of September 2026, the active codebase in `upstream/tectonic` consists of:

```
Total Active Codebase:
  Rust:    49,531 lines (144 files)  ──  28.1% of active codebase
  C/C++:  148,898 lines (158 files)  ──  71.9% of active codebase
  Headers: 22,295 lines (145 files)
```

### Component Status Matrix

| Subsystem | Rust LoC | C/C++ LoC | % Rust | Current Status & Role |
| :--- | :---: | :---: | :---: | :--- |
| **`engine_bibtex`** | 9,126 | 0 | **100%** | **Complete**. Pure-Rust Web2C BibTeX replacement. |
| **`src/` (Driver/CLI)** | 7,121 | 0 | **100%** | **Complete**. Pure-Rust session orchestration, CLI, build pipeline. |
| **`xetex_format`** | 6,406 | 0 | **100%** | **Complete**. Pure-Rust format serialization & deserialization. |
| **`engine_spx2html`**| 4,976 | 0 | **100%** | **Complete**. Pure-Rust HTML/SPX web generation engine. |
| **`xetex_layout`** | 3,377 | 263 | **92.8%** | HarfBuzz text shaping & OpenType layout abstraction. |
| **`bundles`, `io_base`, `xdv`** | 5,592 | 0 | **100%** | **Complete**. VFS, in-memory I/O, bundle resolution, XDV parser. |
| **`bridge_core`** | 1,824 | 1,015 | **64.2%** | C-to-Rust bridge, memory safety bounds, DAP hooks. |
| **`engine_xetex`** | 515 | 49,922 | **1.0%** | **Target 2**: TeX stomach, mouth, macro execution, line breaking. |
| **`pdf_io` & `xdvipdfmx`** | 408 | 71,496 | **0.6%** | **Target 1**: DVI/XDV-to-PDF, font subsetting, CID mappings, PDF streams. |
| **Native C bridges** | 6,864 | 27,887 | ~20% | System HarfBuzz, FreeType2, ICU, Graphite2 native wrappers. |

---

## 3. Mandatory Safety & Governance Gates

By mandate of the Ciel Council of Five (Safety Member veto conditions), no legacy C component in Phases 3 through 5 may be deprecated or removed without passing these seven unbypassable gates:

1. **Gate 1 (Unbypassable CI Blocker)**: The Phase 2 Differential Testing Harness (`tectonic-crater`) must be fully operational in CI as a required status check before any C code replacement.
2. **Gate 2 (Two-Tier Parity Verification)**:
   - **Tier A (Intermediate IR Parity)**: Exact 1:1 bitwise byte-for-byte identity of generated `.xdv` streams compared against the reference C engine.
   - **Tier B (PDF Output Parity)**: Normalized AST content-stream identity and 300+ DPI visual raster diffing (zero pixel deviation on vector glyph positioning, stripping non-deterministic metadata/timestamps).
3. **Gate 3 (Crater Corpus Diversity)**: Parity must be validated against a corpus of $\ge 1,000$ real-world stress documents covering macro edge cases (`expl3`, `tikz/pgf`, `hyperref`, `biblatex`, `microtype`, `beamer`) and complex scripts (Arabic, Devanagari, CJK).
4. **Gate 4 (Fixed-Point Math Invariant)**: Absolute prohibition of IEEE 754 floating-point arithmetic for TeX layout calculations. Strict preservation of 32-bit fixed-point (`scaled_t = 1/65536 pt`) with audited integer overflow behavior.
5. **Gate 5 (Zero `static mut` Invariant)**: Prohibition of process-global `static mut` in all new Rust code. All engine state must reside within a thread-safe, reinstantiable `TeXEngineState` context.
6. **Gate 6 (Unsafe Containment & Miri/ASan CI)**: Transpiled code blocks must be encapsulated behind safe Rust APIs and verified against undefined behavior via AddressSanitizer, UBSan, and Miri.
7. **Gate 7 (Dual-Engine Fallback Gating)**: Migrated subsystems must support shadow dual-execution in testing and retain compile-time/runtime fallback flags (`--use-c-backend`) for at least two stable release cycles.

---

## 4. Phased Engineering Blueprint

```
 Phase 0: Perimeter Virtualization [COMPLETE]
    │
 Phase 1: Deep Observability & DAP Hooks [COMPLETE]
    │
 Phase 2: Differential Testing Harness (`tectonic-crater`) [PREREQUISITE]
    ├── Tier A: Bitwise XDV Parity
    └── Tier B: Normalized PDF AST & Visual Diffing
    │
 ┌──┴──────────────────────────────────────────────────────┐
 │                                                         │
Phase 3a: Image/PDF Probe Decoupling        Phase 4: Subsystem Decoupling
(Extract xetex-pic.c -> dpx-*.h dependency) (Hyphenation trie, string pool, metrics)
 │                                                         │
Phase 3b: Declarative Backend Modernization                │
(pdf-writer, subsetter, ttf-parser)                        │
 └─────────────────────────┬───────────────────────────────┘
                           │
Phase 4.5: Pure-Rust Font & Unicode Decoupling
(rustybuzz, read-fonts, icu4x for pure wasm32-unknown-unknown)
                           │
Phase 5: "TeX Stomach" Transpilation
(xetex0.c -> instantiable TeXEngineState struct automaton)
                           │
Phase 6: Safe Idiomatic Modernization
(Progressive refactoring of unsafe blocks with continuous crater CI)
```

---

### Phase 0: Perimeter Virtualization ✅ COMPLETE
- Replaced POSIX filesystem dependencies with virtualized Rust `IoProvider` (`crates/io_base`).
- Replaced Web2C BibTeX with pure-Rust `engine_bibtex` (9,126 lines).
- Implemented format file serialization in pure-Rust `xetex_format` (6,406 lines).
- Virtualized asset bundles, HTTP fetching, and caching in `crates/bundles`.

### Phase 1: Deep Observability & DAP Interception ✅ COMPLETE
- Surfaced macro expansion context (`MacroExpansionContext`: name, depth, `cur_cmd`, `cur_chr`) from C engine to Rust `DriverHooks`.
- Injected zero-overhead atomic detachment guards (`__builtin_expect(ttstub_is_expansion_hook_active(), 0)`).
- Bound engine stepping directly into `ferrotex-dap` for IDE stepping and inspection.

---

### Phase 2: Differential Testing & Parity Harness (`tectonic-crater`)
**Objective**: Construct the automated, unbypassable testing harness that guarantees 1:1 rendering equivalence.

- **Scope**:
  - Headless test orchestrator compiling a corpus of $\ge 1,000$ documents from arXiv, CTAN, and TeXLive.
  - **Tier A (Bitwise XDV Comparator)**: Decodes and compares `.xdv` opcode streams emitted by the reference C engine vs. the Rust engine.
  - **Tier B (Visual & AST PDF Comparator)**: Normalizes PDF object streams (stripping creation dates, random font subset prefixes, and object IDs) and performs 300 DPI pixel-raster diffing via headless PDFium/Poppler.
  - Knuth canonical TRIP and XeTeX test suite integration.
- **Acceptance Criteria**:
  - 0 opcode drift across all Tier A test cases.
  - 0 pixel divergence on vector text rendering across Tier B visual diffs.
  - Integrated as a required status check in GitHub Actions and local `cargo test --package tectonic-crater`.

---

### Phase 3: Declarative Backend Modernization (~71,000 lines of C)
**Objective**: Eliminate the largest C footprint (`pdf_io` / `xdvipdfmx`) by replacing it with a modern, declarative pure-Rust XDV-to-PDF pipeline.

#### Phase 3a: Image & PDF Dimension Probe Decoupling (Prerequisite)
- **Problem**: `engine_xetex` has a compile-time C linkage dependency on `pdf_io` (`xetex-pic.c` includes `pdf_io/dpx-*.h` for `\XeTeXpicfile` and `\XeTeXpdffile` dimension and page-count queries). Deleting `pdf_io` directly breaks the TeX engine.
- **Solution**: Extract image/PDF header parsing into an independent crate `tectonic_image_probe` (using `image` and `pdf-writer`/`lopdf`) with a minimal C-ABI export, preserving `engine_xetex` build integrity.

#### Phase 3b: Crate-Substituted XDV-to-PDF Pipeline
- **Strategy**: **Avoid a 1:1 mechanical port** of 71k lines of legacy `dpx-*` C code (obsolete PK raster fonts, TPIC specials, ad-hoc Deflate). Substitute with mature, battle-tested Rust crates:
  - `tectonic_xdv::XdvEvents`: Event stream driver (already existing in pure Rust).
  - `pdf-writer`: High-performance, declarative, zero-allocation PDF generation.
  - `subsetter` / `ttf-parser`: Fast OpenType, TrueType, and CFF font subsetting.
  - `image` + `miniz_oxide`: PNG/JPEG/Deflate embedding.
- **Result**: Compresses the migration footprint from 71,000 lines of C to **<15,000 lines of safe, idiomatic Rust**.

---

### Phase 4: TeX Subsystem Decoupling (~15,000 lines of C)
**Objective**: Extract deterministic, standalone submodules out of `engine_xetex` into reusable, testable Rust crates.

- **Submodules to Decouple**:
  1. **Hyphenation Engine (`tectonic_hyphenation`)**: Liang's hyphenation trie algorithm (`patgen` structures). Purely deterministic string-in, hyphenation-points-out.
  2. **String Pool & Table Management (`tectonic_strings`)**: Encapsulate Knuth's `str_pool` and `str_start` tables behind safe, bounds-checked Rust string storage with UTF-16 surrogate handling.
  3. **Shared Font Model (`tectonic_fonts`)**: Unify TFM, OpenType metrics, and font table ingestion to share font descriptors between the engine and the Phase 3 PDF backend.
- **Benefits**: Decoupled submodules immediately improve FerroTeX LSP indexing, completions, and hover analysis without waiting for full stomach transpilation.

---

### Phase 4.5: Pure-Rust Font & Unicode Decoupling (The WASM Enabler)
**Objective**: Replace native C library bridges (`bridge_harfbuzz`, `bridge_freetype2`, `bridge_icu`, `bridge_fontconfig`) with pure-Rust crates.

- **Crate Substitutions**:
  - `bridge_harfbuzz` $\rightarrow$ [`rustybuzz`](https://github.com/RazrFalcon/rustybuzz) (pure-Rust HarfBuzz port).
  - `bridge_freetype2` $\rightarrow$ [`ttf-parser`](https://github.com/RazrFalcon/ttf-parser) / [`read-fonts`](https://github.com/googlefonts/fontations).
  - `bridge_icu` $\rightarrow$ [`icu4x`](https://github.com/unicode-org/icu4x) (modern modular Unicode in Rust).
- **Outcome**: Completely eliminates C system dependencies, enabling clean compilation to `wasm32-unknown-unknown` without Emscripten or Asyncify.

---

### Phase 5: "TeX Stomach" Transpilation (~35,000 lines of C)
**Objective**: Migrate `xetex0.c` into an encapsulated, re-entrant Rust execution automaton.

- **Architecture of `TeXEngineState`**:
  ```rust
  pub struct TeXEngineState<'a> {
      // Memory arrays
      pub mem: Vec<MemoryWord>,
      pub eqtb: Vec<MemoryWord>,
      pub hash: Vec<TwoHalves>,
      
      // Fixed-point state
      pub cur_val: i32,
      pub cur_cmd: u8,
      pub cur_chr: i32,
      pub cur_cs: i32,
      
      // Virtualized I/O & Hooks
      pub io: &'a mut dyn IoProvider,
      pub hooks: &'a mut dyn DriverHooks,
  }
  ```
- **Transpilation Rules**:
  - Preserve exact integer types (`scaled_t = i32`, `halfword = i32`, `quarterword = u8`).
  - Preserve Knuth's exact division, multiplication, and `badness()` routines.
  - Preserve `expand()` and `macro_call()` token dispatch flow.
  - Zero process-global static state; instances are fully re-entrant and thread-safe.

---

### Phase 6: Safe Idiomatic Modernization
**Objective**: Progressively eliminate `unsafe` blocks and raw pointer operations once differential parity is locked.

- Refactor `mem` node arrays into Rust enum variants where layout allows.
- Replace manual linked-list node traversals with safe iterator patterns.
- Continuous verification in CI via Miri to detect any memory safety violations or provenance issues.

---

## 5. Architectural Unlocks for FerroTeX

Completing this roadmap transforms Tectonic from an external CLI tool into an embedded, high-performance compilation runtime:

1. **Instant Keystroke Feedback (<50ms)**:
   - Encapsulated `TeXEngineState` supports **Clone-on-Write (CoW) Preamble Snapshotting**.
   - The document preamble (`\documentclass`, 50+ packages) executes once. Editing body text simply branches off the snapshotted state, eliminating 90% of compile time.
2. **Reversible "Time-Travel" DAP Debugging**:
   - Snapshotting engine state at macro expansion intervals enables genuine `stepBack` execution in VS Code without re-running from scratch.
3. **Hermetic In-Browser Compilation**:
   - A pure `wasm32-unknown-unknown` binary (<8MB compressed) runs directly in web workers with native Fetch/IndexedDB, powering web-based LaTeX editors without servers.
4. **Massive Compilation Concurrency**:
   - Multi-document or multi-pass builds execute across Rayon thread pools in a single process without OS process forks.

---

## 6. Revision History & Council Log

| Date | Revision | Author / Governance | Notes |
| :--- | :---: | :--- | :--- |
| **2026-09-18** | `v1.0.0` | **Ciel Council of Five** | Initial formalization ratified (Docket `council-20260918-tectonic-migration-roadmap.json`, Score: 8.125/10). Established 7 mandatory safety gates, 2-tier parity criteria, Phase 3a decoupling prerequisite, and Phase 4.5 WASM font track. |
