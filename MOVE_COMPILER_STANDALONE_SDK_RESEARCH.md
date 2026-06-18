# Standalone Move Compiler SDK — feasibility research

**Question:** Can the "compile Move source → bytecode" pipeline used by the IOTA
tooling be extracted into a standalone SDK that does **not** depend on this
monorepo? If it must depend on it, what exactly is required? And could it run in
WebAssembly?

**Short answer:** Yes. The entire compilation toolchain already lives in
`external-crates/move/`, a *separate Cargo workspace* with its own `Cargo.lock`
and **zero `iota-*` crate dependencies**. The IOTA-specific wrapper
(`iota-move-build`) only adds publish/validation concerns — and *that* is what
drags in ~31 monorepo crates. The pure compiler does not. A wasm build of the
compiler engine is also viable: it compiles for `wasm32`, and it **runs** under
`wasm32-wasip1` (verified with wasmtime) including the `rayon` code paths.

All numbers and outputs below were produced by building/running throwaway
experiments against this repository.

---

## 1. How the tooling compiles today

The CLI (`crates/iota`) is the "external tool." Its build/publish path is a
four-layer stack:

```
iota CLI (crates/iota, crates/iota-move)
   └─ iota-move-build              ← IOTA-specific wrapper (the monorepo coupling)
        └─ move-package            ← package system: Move.toml/Move.lock, dep graph, git fetch
             └─ move-compiler      ← the actual source→bytecode engine
                  └─ move-binary-format, move-bytecode-verifier, … (siblings)
```

Core call chain in `crates/iota-move-build/src/lib.rs`:

- `BuildConfig::build(path)` → `resolution_graph()` → `move_package`'s
  `resolution_graph_for_package()` (reads the manifest, builds + resolves the
  dependency graph, **shells out to the `git` CLI** to fetch git deps —
  `move-package/src/resolution/dependency_cache.rs`).
- → `build_from_resolution_graph()` →
  `BuildPlan::create(graph).compile_with_driver(|c| c.build())` — this is where
  `move-compiler` turns source into `CompiledModule`s.
- Then IOTA-specific post-processing: `fill_metadata()` (authenticator
  attributes), `verify_bytecode()` (stock Move verifier **plus** the IOTA
  verifier), `gather_published_ids()` (link deps to on-chain IDs).

The bottom two layers are the compiler. Everything above is IOTA glue.

---

## 2. Crates involved, and how many

| Layer | Internal `move-*` crates | Total crates in tree | `iota-*` crates |
|---|---|---|---|
| `move-compiler` (raw engine) | 17 | 181 | **0** |
| `move-package` (full package system) | 23 | 209 | **0** |
| `iota-move-build` (current wrapper) | (same 23) | **588** | **31** |

Notable externals in the compiler tree: `clap`, `serde`, `rayon`, `petgraph`,
`regex`, `codespan-reporting`, `lsp-types`, `vfs`, `walkdir`, `named-lock`,
`toml`, `stacker`, `similar`. There is **no** `tokio`, `reqwest`, `git2`,
`tonic`, or `hyper` — no async runtime and no embedded git (it shells out to the
`git` binary).

The jump from 209 → 588 crates (and 0 → 31 `iota-*`) is *entirely* the IOTA
wrapper. The heaviest items it pulls in: `iota-types`, `iota-verifier-latest`
(→ `iota-adapter-latest`, `move-vm-runtime`, `iota-move-natives-latest`,
`iota-protocol-config`), `iota-package-management` (→ `iota-package-resolver`,
JSON-RPC types), plus `iota-network-stack`, `iota-http`, `iota-tls`,
`iota-metrics`, etc.

---

## 3. Pure Move vs. genuinely IOTA-specific

Everything `iota-move-build` adds on top of `move-package`, and what it actually
needs:

| IOTA addition | Needed to produce bytecode? | Dependency cost |
|---|---|---|
| Set `Flavor::Iota` | Yes (for IOTA syntax) | **None** — the Iota flavor lives *inside* `move-compiler` (`iota_mode` module, `Flavor::Iota`) |
| `IotaPackageHooks` (published-at field, edition check) | Yes | **None heavy** — ~30 LOC trait impl; hooks are optional and default sensibly if unregistered |
| Framework as implicit deps (`implicit_deps`) | Yes (for `use iota::…`) | Move **source**, fetched from `github.com/iotaledger/iota.git` or a local path — not a Rust crate |
| `gather_published_ids` / published-at resolution | No — only for *publish-time* linking | `iota-package-management` (+ resolver, RPC types) |
| IOTA bytecode verifier (`iota_verify_module_unmetered`) | No — extra validation; the node re-checks on publish | `iota-verifier-latest` → **the whole execution layer** (the biggest anchor) |
| Metadata (`fill_metadata`, authenticator attrs) | No — IOTA-specific post-processing | `iota-types` |
| Wrapper types (`ObjectId`, `MovePackage`) | No | `iota-sdk-types`, `iota-types` |

**Key insight:** the things required to *emit correct IOTA bytecode* (flavor,
hooks, framework sources) need **no monorepo Rust crates**. The things that force
the heavy coupling (IOTA verifier, metadata, published-id linking) are about
*validation and publishing*, not compilation.

---

## 4. Experiments and results

All experiments were throwaway crates added to the `external-crates/move`
workspace, built and run, then removed.

### A — minimal engine (`move-compiler` only)

```
OK: compiled 1 module(s)
  module 0x42::demo  ->  163 bytes (bytecode format v6)
```

### B — full package path (`move-package` `BuildConfig`)

```
OK: compiled package with 1 root module(s)
  module demo  ->  94 bytes (bytecode format v6)
```

No `IotaPackageHooks` registered — `move-package`'s default `custom_resolve_pkg_id`
falls back to the manifest package name.

### C — Tier-2 SDK: a real framework-dependent IOTA package, zero `iota-*` crates

A package whose module uses `iota::object::UID` (forcing the Iota flavor *and*
framework linkage), compiled with only `external-crates/move` crates plus a
locally reimplemented `IotaPackageHooks`:

```
OK: 1 root module(s) compiled, 85 dependency module(s) linked
  root module app -> 181 bytes
```

### D — compile from an in-memory filesystem (wasm prerequisite)

```
OK: module 0x99::vfsdemo compiled from MemoryFS -> 104 bytes
```

All file I/O in `move-compiler` is already behind the `vfs` abstraction; when no
root is given it merely defaults to `PhysicalFS::new("/")`.

### E — wasm build + wasm runtime

- `cargo check -p <gauge> --target wasm32-unknown-unknown` **passes** — 154
  crates built for wasm including `rayon`, `stacker`, `regex`,
  `move-bytecode-verifier`. The only source change needed is the standard
  `getrandom` "js" feature shim.
- Built for `wasm32-wasip1` and **run under wasmtime 27.0.0 with no thread
  support enabled**:

```
$ wasmtime run wasm-run.wasm          # NO --wasm threads=y, NO wasi-threads
[wasm] start: building in-memory sources
[wasm] calling Compiler::build()  <-- this hits rayon par_iter
[wasm] WASM-OK: compiled 3 module(s)
  0xAA::a -> 160 bytes
  0xBB::b -> 160 bytes
  0xCC::c -> 160 bytes
$ echo $?
0                                      # deterministic across reruns
```

`move-compiler`'s parser parses files via `rayon`'s `into_par_iter()`
*unconditionally* (`parser/mod.rs:65`), so this run genuinely exercised rayon.
On threadless wasm, thread creation fails at runtime and **rayon falls back to
inline execution rather than panicking**. The module instantiated under plain
wasmtime (which rejects shared-memory imports unless threads are enabled),
proving it is genuinely single-threaded. rayon version: 1.10.0.

**Conclusion on rayon:** it is **not** a runtime blocker on `wasm32-wasip1`. The
earlier assumption that wasm would require `wasm-bindgen-rayon` or source patches
does not hold for WASI.

---

## 5. Proposed standalone SDK

### Tiers

1. **Pure Move compiler SDK (0 IOTA).** Lift `move-package` (+ 22 siblings) or
   just `move-compiler`. ~209 crates, **zero** monorepo coupling. Compiles any
   Move package. These crates originate upstream from the Move language repo and
   are published independently there — extraction is mostly repackaging.

2. **IOTA-flavored compiler SDK (0 monorepo Rust crates — recommended).**
   Tier 1 + (a) set `Flavor::Iota`, (b) a ~30-line `IotaPackageHooks`
   reimplementation, (c) ship/fetch the framework Move sources. Produces
   publish-ready IOTA bytecode. Still **no `iota-*` Rust dependency.**

3. **Full parity with `iota-move-build`.** Only if you also want the IOTA
   verifier + metadata + published-id linking in-process. Requires
   `iota-verifier`, `iota-types`, `iota-sdk-types`, `iota-package-management` →
   the 31-crate / 588-total footprint. Not advisable for an external SDK; the
   IOTA verifier in particular is consensus/protocol-versioned execution-layer
   code that should not be forked.

### Crate manifest sketch (Tier 2)

```toml
[package]
name = "iota-move-compiler"
edition = "2021"

[dependencies]
anyhow = "1"

# The Move toolchain. Today these crates are `publish = false`; pull them via a
# git dep pinned to the iota repo's external-crates/move subtree, OR vendor that
# subtree, OR flip them to publishable. NONE of them depend on iota-* crates.
move-package       = { git = "https://github.com/iotaledger/iota", rev = "<pin>" }
move-compiler      = { git = "https://github.com/iotaledger/iota", rev = "<pin>" }
move-symbol-pool   = { git = "https://github.com/iotaledger/iota", rev = "<pin>" }
move-binary-format = { git = "https://github.com/iotaledger/iota", rev = "<pin>" } # re-export CompiledModule
# (move-package transitively brings the other ~20 move-* crates)
```

### The only IOTA-specific code (verbatim from the working experiment)

```rust
// hooks.rs — the complete IOTA package-system glue, no iota-* deps.
use move_compiler::editions::Edition;
use move_package::{
    package_hooks::{PackageHooks, PackageIdentifier},
    source_package::parsed_manifest::{OnChainInfo, SourceManifest},
};
use move_symbol_pool::Symbol;

pub struct IotaPackageHooks;
impl PackageHooks for IotaPackageHooks {
    fn custom_package_info_fields(&self) -> Vec<String> {
        // Required, or the framework's Move.toml `published-at`/`version` fail to parse.
        vec!["published-at".into(), "version".into()]
    }
    fn resolve_on_chain_dependency(&self, _: Symbol, _: &OnChainInfo) -> anyhow::Result<()> {
        Ok(())
    }
    fn custom_resolve_pkg_id(&self, m: &SourceManifest) -> anyhow::Result<PackageIdentifier> {
        if (!cfg!(debug_assertions) || cfg!(test)) && m.package.edition == Some(Edition::DEVELOPMENT) {
            return Err(Edition::DEVELOPMENT.unknown_edition_error());
        }
        Ok(m.package.name)
    }
    fn resolve_version(&self, _: &SourceManifest) -> anyhow::Result<Option<Symbol>> {
        Ok(None)
    }
}
```

Compile entry point — just `move-package` with two IOTA settings:

```rust
move_package::package_hooks::register_package_hooks(Box::new(IotaPackageHooks));
let mut cfg = move_package::BuildConfig::default();
cfg.default_flavor = Some(Flavor::Iota);          // the only flavor IOTA allows
cfg.implicit_dependencies = framework_deps();     // local-path or git to framework sources
let pkg = cfg.compile_package_no_exit(path, &mut std::io::sink())?;
let modules: Vec<CompiledModule> =
    pkg.root_compiled_units.iter().map(|u| u.unit.module.clone()).collect();
```

### Framework sourcing (the one genuine input you must supply)

The framework is **Move source**, not a Rust crate. Options, in order of
independence:

1. **Vendor** `crates/iota-framework/packages/{move-stdlib,iota-framework,iota-system,stardust}`
   into the SDK (pin per protocol version). Fully offline; recommended.
2. **Git dep** to `github.com/iotaledger/iota.git` (what the node uses via
   `SYSTEM_GIT_REPO`) — needs the `git` binary at runtime.
3. **Local path** (what experiment C used).

---

## 6. WebAssembly

| Concern | Status | Mitigation |
|---|---|---|
| file I/O | non-issue | already behind `vfs`; host fills a `MemoryFS` (experiment D) |
| `getrandom` | needs js feature on `wasm32-unknown-unknown`; native on WASI | add `getrandom` `js` feature for browser; nothing for wasip1 |
| `rayon` runtime | **runs threadless on wasip1 (verified)** | none needed for wasip1; browser via `wasm-bindgen-rayon` if its fallback differs |
| `stacker` (`growing_stack` wraps 132 compiler fns) | compiles; no-op on wasm | raise wasm stack at link time; deep-nesting overflow risk only on pathological input |
| `move-package` (git CLI, `named-lock`, `whoami`, dir-walking) | not for wasm | keep dependency resolution on the host |

**Recommended wasm architecture** — split resolution (host) from compilation
(wasm), which the `vfs` seam makes natural:

```
 Host (JS / native)                         wasm module
 ───────────────────                        ──────────────────────────
 read Move.toml / Move.lock                 move-compiler over a MemoryFS
 fetch git deps, source the framework  ──▶  filled with (path, bytes)
 flatten to a set of source files           → Vec<CompiledModule> (bytes out)
```

Do **not** port `move-package` to wasm; port `move-compiler` and feed it sources
via `MemoryFS`. Tested target was `wasm32-wasip1` + wasmtime 27 (the natural
target for a sandboxed/server-side compiler). The browser target
(`wasm32-unknown-unknown`) compiles but was not run here — rayon's fallback there
is plausibly the same but unverified.

---

## 7. Recommendation

Extract **Tier 2**: the compiler is already monorepo-independent, and a ~30-line
hooks shim plus the Iota flavor plus framework sources is all the IOTA-specific
glue needed to emit publish-ready bytecode. Leave the IOTA verifier, metadata,
and on-chain-ID linking (Tier 3) in/alongside the node, where they belong; let
the SDK either skip them (the node re-validates on publish) or run only the stock
`move-bytecode-verifier`, which is itself in `external-crates/move` with zero
monorepo coupling.

---

## Appendix — reproduction

The experiments were standalone crates added under
`external-crates/move/crates/` (workspace member glob `crates/*`), then removed.

### Minimal engine + full package (experiments A/B)

`Cargo.toml`:

```toml
[package]
name = "compile-poc"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
anyhow.workspace = true
move-compiler.workspace = true
move-package.workspace = true
```

`src/main.rs`:

```rust
use std::{collections::BTreeMap, io::Write, path::Path};

use move_compiler::{
    Compiler, Flags,
    shared::{NumericalAddress, PackageConfig},
};

const SRC: &str = r#"
module 0x42::demo {
    public fun add(a: u64, b: u64): u64 { a + b }
    public fun fib(n: u64): u64 { if (n < 2) n else fib(n - 1) + fib(n - 2) }
}
"#;

fn experiment_a() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join("compile_poc_a");
    std::fs::create_dir_all(&dir)?;
    let src = dir.join("demo.move");
    std::fs::File::create(&src)?.write_all(SRC.as_bytes())?;

    let targets: Vec<String> = vec![src.to_string_lossy().into_owned()];
    let deps: Vec<String> = vec![];
    let named: BTreeMap<String, NumericalAddress> = BTreeMap::new();

    let (_files, units) = Compiler::from_files(None, targets, deps, named)
        .set_flags(Flags::empty())
        .set_default_config(PackageConfig::default())
        .build()?;
    if let Ok((units, _)) = units {
        for u in units {
            let m = &u.named_module.module;
            let mut b = vec![];
            m.serialize_with_version(m.version, &mut b)?;
            println!("module {}::{} -> {} bytes", u.named_module.address, u.named_module.name, b.len());
        }
    }
    Ok(())
}

fn experiment_b() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join("compile_poc_b");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sources"))?;
    std::fs::File::create(root.join("Move.toml"))?.write_all(
        br#"[package]
name = "demo"
edition = "2024"

[addresses]
demo = "0x42"
"#,
    )?;
    std::fs::File::create(root.join("sources").join("demo.move"))?
        .write_all(br#"module demo::demo { public fun add(a: u64, b: u64): u64 { a + b } }"#)?;

    let mut cfg = move_package::BuildConfig::default();
    cfg.silence_warnings = true;
    cfg.install_dir = Some(root.clone());
    let pkg = cfg.compile_package_no_exit(Path::new(&root), &mut std::io::sink())?;
    println!("compiled {} root module(s)", pkg.root_compiled_units.len());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    experiment_a()?;
    experiment_b()
}
```

### Tier-2 framework compile (experiment C)

Add `move-symbol-pool.workspace = true` to deps, register the `IotaPackageHooks`
from §5, then:

```rust
let framework = repo_root.join("crates/iota-framework/packages/iota-framework");
// Move.toml: [dependencies] Iota = { local = "<framework>" }   [addresses] myapp = "0x0"
// sources/app.move:
//   module myapp::app {
//       public struct Counter has key { id: iota::object::UID, value: u64 }
//       public fun value(c: &Counter): u64 { c.value }
//   }
let mut cfg = move_package::BuildConfig::default();
cfg.silence_warnings = true;
cfg.default_flavor = Some(move_compiler::editions::Flavor::Iota);
cfg.install_dir = Some(root.clone());
cfg.skip_fetch_latest_git_deps = true;
let pkg = cfg.compile_package_no_exit(Path::new(&root), &mut std::io::sink())?;
```

### In-memory / wasm compile (experiments D + E)

`Cargo.toml`:

```toml
[package]
name = "wasm-run"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
anyhow.workspace = true
move-compiler.workspace = true
vfs.workspace = true

# Browser only; not needed for wasm32-wasip1:
[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "0.2", features = ["js"] }
```

`src/main.rs`:

```rust
use std::{collections::BTreeMap, io::Write};

use move_compiler::{
    Compiler, Flags,
    shared::{NumericalAddress, PackageConfig},
};
use vfs::{VfsPath, impls::memory::MemoryFS};

fn module_src(addr: &str, name: &str) -> String {
    format!(
        "module 0x{addr}::{name} {{\n    \
         public fun f(x: u64): u64 {{ x + 1 }}\n    \
         public fun g(n: u64): u64 {{ if (n < 2) n else g(n - 1) + g(n - 2) }}\n}}\n"
    )
}

fn main() -> anyhow::Result<()> {
    let fs = VfsPath::new(MemoryFS::new());
    let mut targets = vec![];
    for (addr, name) in [("aa", "a"), ("bb", "b"), ("cc", "c")] {
        let p = format!("{name}.move");
        fs.join(&p)?.create_file()?.write_all(module_src(addr, name).as_bytes())?;
        targets.push(p);
    }
    let deps: Vec<String> = vec![];
    let named: BTreeMap<String, NumericalAddress> = BTreeMap::new();

    // Some(vfs_root) routes all file reads through the MemoryFS.
    let (_files, units) = Compiler::from_files(Some(fs), targets, deps, named)
        .set_flags(Flags::empty())
        .set_default_config(PackageConfig::default())
        .build()?;
    if let Ok((units, _)) = units {
        for u in units {
            let m = &u.named_module.module;
            let mut b = vec![];
            m.serialize_with_version(m.version, &mut b)?;
            println!("{}::{} -> {} bytes", u.named_module.address, u.named_module.name, b.len());
        }
    }
    Ok(())
}
```

Build & run for WASI:

```sh
rustup target add wasm32-wasip1
cargo build -p wasm-run --target wasm32-wasip1
wasmtime run target/wasm32-wasip1/debug/wasm-run.wasm
```
