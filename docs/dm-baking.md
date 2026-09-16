# DM appearance baking

The compiler retains procedure bodies in `ir::Module`, lowers them to a `codegen::Module`, and runs
that stack bytecode in the headless `vm` crate. Appearance baking builds a runtime world from a map,
executes a small profile-owned DM interface, and returns an `AppearanceDelta` for each placed atom.
Baking does not write to the map, its dictionary, or editor history.

The compiler driver exposes the complete pipeline:

```sh
cargo run --release --bin rmdc -- bake <environment.dme> <map.dmm>
cargo run --release --bin rmdc -- bake <environment.dme> <map.dmm> --summary --check-edit
```

The regular output lists each placement's coordinates, instance ID, and resolved `icon_state`.
`--summary` suppresses those rows. `--check-edit` removes and restores the first wall, times both
updates, and verifies that the complete derived appearance layer returns to its original state.

## Compilation and profiles

The compiler prelude declares three hooks:

```dm
/proc/demir_initialize(atom/target)
/proc/demir_light(atom/target)
/proc/demir_bake(atom/target)
```

Profiles live in `prelude/profiles/` and are preprocessed after the project's entry file. This lets a
profile use macros, types, variables, and procedures declared by the project. Each postlude is
drained as its own token stream so indentation and end-of-file state cannot leak between files.

The default profile makes all three hooks no-ops. Automatic codebase detection and the tgstation,
Goonstation, and Vanderlin profiles are separate follow-up work. `DM_PROFILE` can replace the
selected embedded profile with a file on disk.

Baking defines `__DEMIR_BAKE__` before the normal prelude. `prelude/demir.dm` uses it to disable the
`FASTDMM` and `SPACEMAN_DMM` compatibility defines during a bake. This exposes normal initializer
values to profile code instead of editor-specific preview values. Regular compilation still defines
the compatibility macros.

The prelude also declares a codebase-neutral static lighting schema on `/atom`. Profiles may fill it
through `demir_light`; harvesting, solving, and drawing that data are later steps.

## Runtime model

`vm::bake::Bake` owns a `Runtime`, the placed atoms, runtime objects, the world position index,
derived appearances, diagnostics, and the incremental cache. `Atom` is deliberately independent of
the map crate:

```rust
pub struct Atom {
    pub instance: u64,
    pub ty: TypeId,
    pub position: Position,
    pub vars: Vec<(Identifier, Value)>,
}
```

The caller assigns stable instance IDs and translates map prefabs into this form. Areas with the same
type and map-variable overrides share one runtime object while keeping all of their placement IDs.
Turfs, areas, and movable contents are linked through the runtime world before any DM executes.

A full bake has four ordered stages:

1. **Instantiate** creates objects, applies constant map overrides, and links each map cell.
2. **Initialize** calls `demir_initialize`, or falls back to `Initialize(TRUE)` when that hook has no
   body. Shared areas initialize once.
3. **Light** calls `demir_light` once per runtime object so later lighting code can harvest the
   neutral schema.
4. **Smooth** calls `demir_bake` for each placement and exports appearance changes.

Initialization uses the VM's normal transaction behavior and commits only when it succeeds. Each
smoothing call starts a separate heap journal. The baker runs the hook, gathers the target and any
changed neighboring objects, recursively exports their appearance values, and then rolls the journal
back. A failed preview exports nothing, so repeated bakes cannot accumulate runtime writes or
overlays.

Exported appearance fields are limited to renderable scalar values plus nested `overlays` and
`underlays`. Text entries in those lists become floating appearance deltas with BYOND-compatible
defaults. Object graphs are capped at depth 32 and share the VM allocation/instruction budget, so a
cyclic or exponentially expanding overlay graph becomes a `Memory` fault instead of consuming
unbounded work.

Faults are grouped by procedure, source location, and kind. Replacing an atom removes its previous
fault before another attempt. Unsupported operations, sandbox violations, instruction exhaustion,
call-depth exhaustion, and allocation exhaustion all remain visible to the caller.

## Caching and edits

Eligible full-load previews are cached using the atom type, map overrides, initialized variables,
the surrounding 3 by 3 cell signatures, world dimensions, and the current cache epoch. A preview
that reads coordinates or randomness additionally uses the atom position. Global scans and other
nonlocal reads mark the preview unsafe to memoize. The cache holds at most 50,000 entries.

Each placement's fingerprint is a 64-bit hash of its map overrides, in file order, followed by its
initialized runtime variables sorted by name. Runtime values hash through the same key that list
lookup uses, so values that compare equal in DM share a fingerprint. Lists and objects hash by
identity rather than contents, which makes a placement holding one a cache miss rather than a stale
hit. Hashes are only compared within one process.

`Bake::update` accepts replacement atoms and removed instance IDs. It relinks changed cells,
initializes new objects, and rebakes the surrounding 3 by 3 by 3 neighborhood. The vertical extent is
needed by codebases with pipes or other structures that connect between z levels. Incremental edits
bypass full-load cache reuse through the epoch, avoiding results derived from stale runtime globals.

Appearance effects on other placements are stored as contributions from their source instance.
Removing or replacing a source first removes every contribution it produced, then recomposes only
the affected target appearances in stable source-ID order. This also preserves updates made through
shared area objects.

## Sandbox and limits

Every call receives an instruction budget, call-depth limit, allocation budget, and maximum text
size. The default instruction budget is 100,000 operations, call depth is 48, allocations are capped
at 100,000 units, and an individual text result is capped at 1 MiB. Randomness is seeded from the
source type and coordinates, so rebuilding or removing and restoring the same atom is repeatable.

File access, native libraries, networking, sleep, spawn, timers, and interactive input remain
blocked. DM `catch` can handle DM throws but cannot swallow sandbox or resource-limit faults. World
time and tick usage are deterministic, and no client or subsystem loop runs.

## Validation

The VM tests cover neighborhood smoothing, incremental remove/restore, deterministic random results,
instance-variable cache separation, list-backed neighbor overlays, rollback, and bounded recursive
appearance export. The compiler-driver check exercises preprocessing, semantic analysis, bytecode
generation, map translation, the four bake stages, summary output, and incremental restoration:

```sh
cargo test --workspace
cargo run --release --bin rmdc -- bake examples/env/test.dme examples/env/test.dmm --summary --check-edit
```

Editor and viewer appearance integration, codebase-specific profiles, lighting harvest and solving,
and the renderer lighting pass are intentionally implemented by later stack entries.
