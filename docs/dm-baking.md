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

## Profiles

The compiler prelude declares four hooks without bodies:

```dm
/proc/demir_initialize()
/proc/demir_prepare(atom/target)
/proc/demir_light(atom/target)
/proc/demir_bake(atom/target)
```

A codebase opts in by shipping a profile, a DM file that gives one or more of them a body. The
profile lives in the codebase and is included from its `.dme` like any other file, wrapped so that
only a bake compiles it:

```dm
#ifdef __DEMIR_BAKE__

/proc/demir_bake(atom/target)
	if(istype(target, /turf/closed/wall))
		var/turf/closed/wall/wall = target
		wall.smooth_icon()

#endif
```

BYOND never defines `__DEMIR_BAKE__`, so the file compiles to nothing there, and a load with baking
off sees exactly the tree BYOND does. Because the profile is part of the codebase, it can use every
macro the codebase defined before it, and the codebase maintains it alongside the code it calls.
`examples/env/profile.dm` is a minimal one.

A codebase without a profile bakes nothing. No bytecode is generated, no DM runs, and every atom
draws from its static appearance. rmd never calls the game's own `Initialize()` either. Persistent
target-specific preparation, including any call to `target.Initialize(TRUE)`, belongs in
`demir_prepare`. Appearance mutations belong in `demir_bake`.

The editor compiles two views of a codebase when baking is enabled. Its editor tree defines
`__DEMIR_COMPAT__`, `FASTDMM`, and `SPACEMAN_DMM`, preserving mapping-only icons and types. Its bake
program instead defines `__DEMIR_BAKE__`; `prelude/demir.dm` leaves the compatibility defines off so
profile code sees normal runtime initializers. Both preprocessing passes share cached source text,
but keep independent token streams, type IDs, and source-file tables.

The prelude also declares a codebase-neutral static lighting schema on `/atom`. Profiles may fill it
through `demir_light`. The baker harvests those fields into a dense, z-major corner lightmap. The
renderer bilinearly samples the four corners of each tile and applies the result to the map scene.

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

A full bake has five ordered stages:

1. **Instantiate** creates objects, applies constant map overrides, and links each map cell.
2. **Initialize** calls `demir_initialize()` once after the complete runtime world is linked.
3. **Prepare** calls `demir_prepare` once per runtime object and commits successful setup for use by
   neighboring previews.
4. **Light** calls `demir_light` once per runtime object, harvests the neutral schema for every
   placement, and solves each z level's shared lighting corners.
5. **Smooth** calls `demir_bake` for each placement and exports appearance changes.

Initialization uses the VM's normal transaction behavior and commits only when it succeeds. A fault
is reported once and stops the remaining stages, leaving every atom on its static appearance.
Preparation also commits when it succeeds; a per-object fault excludes that object from later hooks.
Each smoothing call starts a separate heap journal. The baker runs the hook, gathers the target and
any changed neighboring objects, recursively exports their appearance values, and then rolls the
journal back. A failed preview exports nothing, so repeated bakes cannot accumulate runtime writes or
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

Eligible full-load previews are cached using the atom type, map overrides, runtime variables,
the surrounding 3 by 3 cell signatures, world dimensions, and the current cache epoch. A preview
that reads coordinates or randomness additionally uses the atom position. Global scans and other
nonlocal reads mark the preview unsafe to memoize. The cache holds at most 50,000 entries.

Each placement's fingerprint is a 64-bit hash of its map overrides, in file order, followed by its
runtime variables sorted by name. Runtime values hash through the same key that list
lookup uses, so values that compare equal in DM share a fingerprint. Lists and objects hash by
identity rather than contents, which makes a placement holding one a cache miss rather than a stale
hit. Hashes are only compared within one process.

`Bake::update` accepts replacement atoms and removed instance IDs. It relinks changed cells, applies
the prepare and light hooks to new objects, and rebakes the surrounding 3 by 3 by 3 neighborhood. It
reuses the runtime initialized by the full bake and never reruns `demir_initialize()`. The vertical
extent is needed by codebases with pipes or other structures that connect between z levels.
Incremental edits bypass full-load cache reuse through the epoch, avoiding results derived from stale
runtime globals. The update reports only the edited instances and the neighbors whose composed
appearance actually changed.

The lightmap keeps its corner samples, per-cell blockers and fullbright/ambient state, and a bucketed
index of sources between solves. A changed light re-solves only the corners inside its reach. A
blocker whose cell flips re-solves the reach of every source that covers it, and a fullbright or
ambient cell re-solves its own corners plus any adjacent edge-only source. Sources are summed in
ascending instance order in both paths, so an incremental solve matches a full one bit for bit. The
update reports the contiguous tile range it rewrote separately from changed appearances.

Appearance effects on other placements are stored as contributions from their source instance.
Removing or replacing a source first removes every contribution it produced, then recomposes only
the affected target appearances in stable source-ID order. This also preserves updates made through
shared area objects.

## Editor and viewer

Both applications bake by default. `DM_BAKE=0` turns baking off for one run. The editor's
**DM Baking** settings tab turns it off and enables perspective editor walls. Both take effect on the
next codebase load.

`Environment::tree` is the compatibility view used by editor tools and static rendering.
`Environment::bake_program` holds the separate runtime tree, bytecode, and source-file table. It is
`None` with baking off, for a codebase without a profile, and when bytecode generation fails. A
failure is reported with the load diagnostics and leaves the map drawn from static appearances.

`editor::bake` translates placed prefabs into `vm::bake::Atom`s, keyed by `PrefabInstanceId`. Each
open map owns its bake. A new environment, a newly opened map, or a change in the number of z levels
bakes the whole map again. An edit, undo, or redo goes through `Bake::update`, and the sprites of
every placement it reports are rebuilt. Hiding a type rebuilds sprites from the cached bake without
running any DM.

Frame building applies fields changed by baking over the compatibility tree's static appearance.
This lets smoothing replace `icon_state` while an untouched atom keeps its mapping-only icon. A type
that exists only under a compatibility define remains drawable but is omitted from the runtime
world. Type IDs never cross between the two views; placed prefabs are resolved by path in each tree.
Overlay and underlay deltas become extra sprites owned by the placement, drawn in list order around
it. They inherit the owner's icon, dir, offsets, and floating layer and plane. Color and alpha
multiply with the owner's unless the overlay sets `RESET_COLOR` or `RESET_ALPHA`.

When a profile defines `demir_light`, the map canvas always applies its baked lighting. The lighting
pass runs after map sprites and before area outlines, selection feedback, and placement previews, so
editor feedback and previews remain readable. Lighting has its own revision and GPU update range;
ordinary appearance edits do not re-upload the lightmap.

Palette thumbnails resolve statically. An atom whose static `icon_state` is missing from its sheet,
which is how smoothed walls are declared, is baked alone in a one cell world at load, and the
derived appearance is used for its thumbnail. The placement preview still resolves statically.

The settings tab lists grouped bake faults. It refreshes after every full bake.

## Sandbox and limits

Every call receives an instruction budget, call-depth limit, allocation budget, and maximum text
size. The default instruction budget is 100,000 operations, call depth is 48, allocations are capped
at 100,000 units, and an individual text result is capped at 1 MiB. Randomness is seeded from the
source type and coordinates, so rebuilding or removing and restoring the same atom is repeatable.

File access, native libraries, networking, sleep, spawn, timers, and interactive input remain
blocked. DM `catch` can handle DM throws but cannot swallow sandbox or resource-limit faults. World
time and tick usage are deterministic, and no client or subsystem loop runs.

Profiles may write debug messages with `world.log << value`. The compiler driver and viewer forward
each line to stderr with a `DM:` prefix. The editor sends it through its normal logger, which writes
to stderr where available and to `latest.log` on Windows. Other output targets remain blocked.

## Validation

The VM tests cover neighborhood smoothing, incremental remove/restore, deterministic random results,
instance-variable cache separation, list-backed neighbor overlays, rollback, bounded recursive
appearance export, corner lighting, blockers, ambient/fullbright cells, and incremental lightmap
restoration. The compiler-driver check exercises preprocessing, semantic analysis, bytecode
generation, map translation, the five bake stages, summary output, and incremental restoration:

```sh
cargo test --workspace
cargo run --release --bin rmdc -- bake examples/env/test.dme examples/env/test.dmm --summary --check-edit
```

The editor tests cover whole-map baking without changing map bytes, overlay sprites, incremental
sprites matching a full rebuild, undo and redo through the bake, movable smoothing, standalone
thumbnails, hiding a type without rebaking, and that only a codebase with a profile bakes.
