# demir

## Introduction

demir is a compiler backend for the DreamMaker language. The name means Dream IR, short for
DreamMaker intermediate representation. The compiler preprocesses and parses DM source, builds an
object tree and intermediate representation, then generates stack bytecode.

rmd uses demir's object tree for editor tools and static rendering. It can also run generated
bytecode while it displays a map. A codebase supplies an optional profile that describes its mapping
behavior. The bake runtime builds a limited world and calls the profile for each placed atom.

The result is a derived map view. It can contain resolved appearances, lighting, connections, and
highlights. The editor also uses profile data for codebase-specific tools. A bake does not change
the map, its dictionary, or the editor history.

Profiles are optional. A codebase without a profile uses static appearances and does not generate
bake bytecode. Compilation or runtime faults also leave affected atoms on their static appearances.

The compiler driver can run a complete bake without the editor:

```sh
cargo run --release --bin rmdc -- bake <environment.dme> <map.dmm>
cargo run --release --bin rmdc -- bake <environment.dme> <map.dmm> --summary --check-edit
```

Normal output includes each placement, its instance ID, and its resolved `icon_state`. The
`--summary` flag hides those rows. The `--check-edit` flag removes and restores the first wall. It
then checks that the derived view returns to its original state.

## Compiler

The editor compiles two views of a codebase when users enable appearance baking.

The compatibility view defines `__DEMIR_COMPAT__`, `FASTDMM`, and `SPACEMAN_DMM`. Editor tools and
static rendering use this view. It preserves mapping-only types, icons, and other compatibility
code.

The bake view defines `__DEMIR_BAKE__`. It omits the mapping compatibility defines and includes the
profile code. The compiler keeps procedure bodies in its intermediate representation. Code
generation converts them to stack bytecode for the virtual machine.

Both views share cached source text. They keep separate tokens, type IDs, source locations, and
object trees. Code must resolve placed types by path when it moves data between the views.

Some build tools pass defines outside the `.dme` file. A codebase must declare required runtime
defines inside its bake branch. The declarations must appear before the generated include block.

The compiler finds every subtype of `/datum/demir`. Every subtype is selectable, including a base
profile and variants derived from it. Exactly one subtype must directly set `default = TRUE`. The
default marker is not inherited for selection purposes, so a debug subtype does not become a
second default merely because its parent is the default profile.

The declared default is used by the viewer, compiler driver, and the editor's first load. The
editor can request another profile by its complete type path. A missing remembered path falls back
to the declared default. Zero or multiple explicit defaults are profile diagnostics and disable
bake bytecode generation until the codebase fixes the declaration.

The bake view produces no bytecode when the codebase has no profile. Preprocessor, analysis, and
code generation faults appear with the load diagnostics. The editor continues to use the
compatibility view.

## Virtual machine

The editor translates every placed prefab into a runtime atom:

```rust
pub struct Atom {
    pub instance: u64,
    pub ty: TypeId,
    pub position: Position,
    pub vars: Vec<(Identifier, Value)>,
}
```

The instance ID stays stable across editor updates. The type belongs to the bake object tree. The
variable list contains map overrides. The bake runtime links areas, turfs, and movable contents
before it runs profile code.

A full bake runs seven stages:

1. **Instantiate** creates runtime objects, applies map overrides, and links map cells.
2. **Initialize** creates the profile and calls its `New()` procedure once.
3. **Prepare** calls `prepare` for each runtime object and keeps successful changes.
4. **Connections** collects profile-defined endpoints and matches their channels.
5. **Highlights** collects the regions declared for each placement.
6. **Light** calls `light`, reads the neutral lighting fields, and solves each z level.
7. **Smooth** calls `bake` and exports the resulting appearance changes.

The bake runtime keeps the profile state created by `New()`. It also keeps successful changes from
`prepare` and `light`. Connection, highlight, and appearance previews run in temporary
transactions. It exports their results and then restores the runtime heap.

An appearance can change renderable scalar fields, overlays, and underlays. Exported appearances
can also carry emissive masks, emissive blockers, and overlay light. Export depth and runtime
budgets limit cyclic or expanding object graphs.

The bake runtime caches safe full-load appearance previews. Local state and the surrounding 3 by 3
cells form the main cache input. Reads of wider global state disable reuse. Reads of coordinates or
random values add the placement position to the cache input.

An edit replaces or removes atoms in the existing runtime. The bake runtime relinks changed cells
and updates the affected 3 by 3 by 3 neighborhood. It also updates connections and lighting. The
editor redraws only placements whose composed appearance changed.

## Profiles

A profile is a DM subtype included from the codebase's `.dme` file. Guard it so BYOND does not
compile the profile:

```dm
#ifdef __DEMIR_BAKE__

/datum/demir/example
	default = TRUE

	bake(atom/target)
		if(istype(target, /turf/closed/wall))
			target.icon_state = "wall"

#endif
```

The profile instance is the `src` value for each hook. Store persistent profile settings in its
variables. Use `New()` for setup and call the parent implementation when the profile inherits from
another profile. Ordinary procedures can get the active profile through `demir_profile()`.

The profile has six hooks:

| Hook                       | Purpose                                                 |
| -------------------------- | ------------------------------------------------------- |
| `prepare(atom/target)`     | Prepare persistent runtime state for one object.        |
| `connections(atom/target)` | Declare named connection channels and endpoint roles.   |
| `highlights(atom/target)`  | Declare regions that the editor can shade.              |
| `light(atom/target)`       | Fill the neutral lighting fields on an atom.            |
| `bake(atom/target)`        | Change the appearance of a placement and its neighbors. |
| `ui(atom/target)`          | Draw the profile panel for the current editor frame.    |

`connections` returns an associative list. Each text key is a channel. Each value contains
`DEMIR_CONNECTION_SOURCE`, `DEMIR_CONNECTION_TARGET`, or both flags. The bake runtime connects placements that
share a channel and have complementary roles.

`highlights` returns a list of associative descriptors. A descriptor can define a rectangle or a
list of tile offsets. It can also set `color`, `fill`, `outline`, `when`, and `label`. The `when`
field combines the selected, hovered, and always flags.

`light` writes the `demir_light_*` fields declared on `/atom`. The schema supports point and cone
sources, blockers, ambient light, fullbright cells, and appearance-based light roles. See
[`prelude/demir.dm`](../prelude/demir.dm) for the complete field list.

`ui` can call the `imgui_*` procedures declared in the prelude. The editor retains widget values
between frames. A frame keeps profile writes only when a user interaction changes the heap.

Call `demir_define_group` from `New()` to assign type subtrees to profile-owned bit groups. A UI
change can pass those bits to `demir_rebake`. This limits new work to the affected kinds and type
groups. A UI change has no map effect until it requests a rebake.

Call `demir_node_group` from `New()` to enable the Node tool for a type subtree. Each registration
can name blocker types that the cardinal router cannot cross.

The prelude is the complete DM interface. It defines every hook, helper, flag, lighting field, and
UI procedure. The editor embeds the example integrations so they can be forced without changing a
codebase. The files remain usable as templates for codebase-owned profiles:

- [`examples/env/profile.dm`](../examples/env/profile.dm) is a small test profile.
- [`examples/profiles`](../examples/profiles) contains integrations for CMSS13, Goonstation,
  tgstation, and Vanderlin.

## Editor and viewer

The editor and viewer enable appearance baking by default. Set `DM_BAKE=0` to disable it for one
run. The editor's **Run DM appearance baking** setting applies on the next codebase load.

The editor's **Compiler > Codebase** setting lists every profile in the loaded codebase and marks
the declared default. Choosing another profile asks for confirmation, then reloads the codebase
and rebakes its open maps without closing them or discarding unsaved map edits. The choice is
remembered per environment. Choosing the declared default clears the override so later changes to
the codebase default take effect.

**Forced bundled profile** can inject one of the embedded integrations after the codebase and use
it even when the codebase declares another default. Required bake-view defines, such as tgstation's
`CBT`, are injected before the environment. Forced choices are remembered per environment. Setting
the option back to **None - use codebase profile** restores the remembered codebase profile.

Each open map owns one bake. A new environment, a new map, or a changed z-level count starts a full
bake. Edits, undo, and redo use incremental updates. Hiding a type rebuilds sprites from cached bake
data and does not run DM again.

The renderer applies each appearance delta over the compatibility view. Unchanged atoms keep their
mapping appearance. Overlays and underlays become extra sprites owned by the placement. They inherit
the owner's appearance fields unless they set their own values.

When a profile defines `light`, the canvas applies the solved lightmap after map sprites. Editor
feedback and placement previews render after the lightmap. Lighting uses a separate revision, so an
appearance-only edit does not upload the lightmap again.

Connections appear as selection guides. Cross-level guides receive a z-level label. Highlights can
appear for the selected atom, the hovered atom, or at all times. The editor clips their tiles to the
active z level and visible map region.

A profile UI uses the editor dockspace. UI writes can request new appearances, lighting, or
highlights. The editor delays a request while the user drags a widget, then runs it after release.

The Node tool appears when the profile registers a node group. It follows cardinal connections and
routes around registered blockers. A drag is one undoable edit. Cancelled or unreachable routes
restore every touched tile.

## Sandbox and limits

The virtual machine applies fixed default limits to each hook call:

| Resource                 | Default limit |
| ------------------------ | ------------: |
| Instructions             |       100,000 |
| Call depth               |            48 |
| Allocations              | 100,000 units |
| Text result              |         1 MiB |
| Constant expansion depth |           128 |

A UI frame allows 4,096 commands and 16 nested windows or nodes. Labels can contain 128 characters.
Text values can contain 1,024 characters. One atom can declare 16 highlights. One highlight can
contain 16,384 tiles and a 64-character label.

The VM blocks file access, native libraries, networking, sleep, spawned work, timers, and
interactive input. DM `catch` can handle DM throws. It cannot consume sandbox or resource faults.

Random values use a seed based on the target atom's type and coordinates. Repeating a bake for the
same atom gives the same random sequence. World time and tick usage are deterministic.

Profiles can write diagnostic text with `world.log << value`. The compiler driver and viewer write
each line to standard error with a `DM:` prefix. The editor sends the line to its normal log.

Faults include the procedure, source location, and kind. The editor groups repeated faults. A failed
appearance preview exports no changes. A failed `New()` call stops the remaining bake stages.

## Development

Run the workspace tests and the compiler driver check after changes to demir:

```sh
cargo test --workspace
cargo run --release --bin rmdc -- bake examples/env/test.dme examples/env/test.dmm --summary --check-edit
```

The tests cover profile selection, world setup, transactions, appearance export, cache safety,
incremental edits, lighting, connections, highlights, UI state, and Node tool integration. The
compiler driver check covers preprocessing, analysis, bytecode generation, map translation, all
seven bake stages, and incremental restoration.

The summary reports changed appearances, cache hits, faults, connections, highlights, and lighting.
Use it to check that a profile reaches the expected placements.
