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

The compiler prelude declares one type with six hooks, none of them with a body:

```dm
/datum/demir
	proc/bake(atom/target)
	proc/prepare(atom/target)
	proc/light(atom/target)
	proc/connections(atom/target)
	proc/highlights(atom/target)
	proc/ui(atom/target)
```

A codebase opts in by shipping a profile, a DM file declaring one subtype of `/datum/demir` that
gives one or more of them a body. The profile lives in the codebase and is included from its `.dme`
like any other file, wrapped so that only a bake compiles it:

```dm
#ifdef __DEMIR_BAKE__

/datum/demir/mycodebase/bake(atom/target)
	if(istype(target, /turf/closed/wall))
		var/turf/closed/wall/wall = target
		wall.smooth_icon()

#endif
```

rmd builds the subtype once per bake and calls every hook on that instance, so a profile keeps what
it needs to remember in its own vars rather than in globals, and `..()` lets a hook chain to the one
it overrides. `/datum/New()` is where a profile sets itself up; it runs once, before every other
hook. A proc that is not itself a hook reaches the same instance through `demir_profile()`.

A codebase may carry several profiles, as long as one is more derived than the rest: rmd bakes with
the subtype of `/datum/demir` that nothing else inherits from. Two unrelated profiles are an error
naming both, because only the codebase can say which one it meant.

BYOND never defines `__DEMIR_BAKE__`, so the file compiles to nothing there, and a load with baking
off sees exactly the tree BYOND does. Because the profile is part of the codebase, it can use every
macro the codebase defined before it, and the codebase maintains it alongside the code it calls.
`examples/env/profile.dm` is a minimal one.

A codebase without a profile bakes nothing. No bytecode is generated, no DM runs, and every atom
draws from its static appearance. rmd never calls the game's own `Initialize()` either. Persistent
target-specific preparation, including any call to `target.Initialize(TRUE)`, belongs in `prepare`.
Appearance mutations belong in `bake`.

`connections` lets a profile expose codebase-specific links without teaching the editor about
the codebase. It returns an associative list whose text keys are opaque connection channels and whose
values use `DEMIR_CONNECTION_SOURCE`, `DEMIR_CONNECTION_TARGET`, or both flags. The baker connects
atoms with the same channel and complementary roles. The hook runs after preparation in a rolled-back
transaction, so it can inspect prepared runtime values but cannot mutate the persistent world.

The editor compiles two views of a codebase when baking is enabled. Its editor tree defines
`__DEMIR_COMPAT__`, `FASTDMM`, and `SPACEMAN_DMM`, preserving mapping-only icons and types. Its bake
program instead defines `__DEMIR_BAKE__`; `prelude/demir.dm` leaves the compatibility defines off so
profile code sees normal runtime initializers. Both preprocessing passes share cached source text,
but keep independent token streams, type IDs, and source-file tables.

Build defines supplied outside the `.dme` must be restored near the top of the file for the bake
view. For example, tgstation's build tool supplies `CBT`, which makes `MAP_SWITCH` select runtime
icons. Its `.dme` therefore defines `CBT` under `#ifdef __DEMIR_BAKE__` after the required
`genesis_call.dme` include and before the generated include block.

The prelude also declares a codebase-neutral static lighting schema on `/atom`. Profiles may fill it
through `light`. The baker harvests those fields into a dense, z-major corner lightmap. The
renderer bilinearly samples the four corners of each tile and applies the result to the map scene.
Nested appearances marked `demir_emissive` contribute their alpha to the emissive mask without
drawing their mask texture into the scene. `demir_overlay_light` similarly routes an appearance to
the additive or subtractive overlay lightmap instead of the scene color. Profiles can move a static
source independently of its sprite with `demir_light_offset_x` and `demir_light_offset_y`, measured
in tile units.

`highlights` lets a profile shade tiles without teaching the editor what they mean. It returns
a list of descriptors, each an associative list; unknown keys are ignored, so a profile may carry
keys a newer rmd reads, and a trailing comma is tolerated because it is ordinary DM style.

| key                                 | meaning                                                                         |
| ----------------------------------- | ------------------------------------------------------------------------------- |
| `"x"`, `"y"`, `"width"`, `"height"` | a rectangle, offset in tiles from the atom's own tile                           |
| `"tiles"`                           | `list(list(x, y), ...)` offsets, for a shape that is not a rectangle            |
| `"color"`                           | any DM color, default orange                                                    |
| `"fill"`                            | wash opacity from 0 to 1, default `0.12`                                        |
| `"outline"`                         | draw the marching border, default on                                            |
| `"when"`                            | `DEMIR_HIGHLIGHT_SELECTED`, `_HOVERED`, `_ALWAYS`, combinable; default selected |
| `"label"`                           | text drawn above the region                                                     |

Offsets locate tiles relative to the atom, so an atom standing inside its own region uses negative
ones. Tiles outside the world are dropped and a descriptor covering nothing is discarded. One
highlight is capped at 16384 tiles, an atom at 16 highlights, and a label at 64 characters, so a
runaway hook costs a diagnostic rather than the editor's memory. The hook runs in a rolled-back
transaction like `connections`, so it can read prepared runtime values but cannot mutate the
world. The baker records which sides of each tile face outward, which is what lets a non-rectangular
shape draw one continuous border.

tgstation's profile uses it to show what area a `/obj/docking_port` covers, deriving a mobile port's
extent from the shuttle map's own bounds the way `calculate_docking_port_information()` does.

`ui` draws the profile's own editor panel. It is the one hook that does not belong to a bake
stage: the editor calls it once per frame with the selected atom, or null. The `imgui_*` procs are
blocked everywhere else:

| proc                                        | answers                    |
| ------------------------------------------- | -------------------------- |
| `imgui_begin(label)`, `imgui_end()`         | whether the window is open |
| `imgui_tree(label)`, `imgui_tree_end()`     | whether the node is open   |
| `imgui_collapsing_header(label)`            | whether the header is open |
| `imgui_button(label)`                       | whether it was pressed     |
| `imgui_checkbox(label, checked)`            | the checked state          |
| `imgui_radio(label, active)`                | whether it was picked      |
| `imgui_slider(label, value, min, max)`      | the value                  |
| `imgui_drag(label, value, speed, min, max)` | the value                  |
| `imgui_input_text(label, value)`            | the text                   |
| `imgui_text(text)`                          | nothing                    |
| `imgui_text_colored(color, text)`           | nothing                    |
| `imgui_separator(label)`                    | nothing                    |
| `imgui_same_line()`                         | nothing                    |
| `imgui_dockspace()`                         | the editor's dockspace     |
| `imgui_set_next_window_dock(dockspace)`     | nothing                    |
| `imgui_set_next_window_size(width, height)` | nothing                    |
| `demir_rebake(kinds, groups)`               | nothing                    |

`demir_define_group(group, type)` is called from `New()` rather than the panel. It puts a type and
everything under it in a group, which is a bit whose meaning is the profile's own; rmd only matches
them. A type may join several groups, and a group may name as many types as it likes.

`demir_node_group(subtype, blocker)` is also called from `New()`. It enables the editor's Node tool
for `subtype` and every type below it. `blocker` may be one type or a list of types; every blocker
and its descendants are tiles the cardinal router cannot cross. Repeat the declaration to add more
blockers, or pass null for none. Invalid list entries reject that declaration without partially
adding its valid entries. When registrations overlap, the most-derived matching subtype controls
the object:

```dm
demir_node_group(/obj/machinery/pipe, list(/turf/closed, /obj/structure/window))
```

`demir_profile()` answers with the profile instance from anywhere, which is how a proc that is not
itself a hook, and so has some atom as its own `src`, reads what the panel wrote:

```dm
/obj/structure/cable/demir_underfloor_shown()
	var/datum/demir/tgstation/profile = demir_profile()
	return profile.show_cables
```

A widget is identified by the path of labels enclosing it, so the same label under two nodes stays
two widgets and a repeated one under the same node keeps a stable identity across frames. Pair
`imgui_tree_end()` with an open node only; the baker closes a collapsed one, and closes whatever a
profile that returns early left open, so the editor never replays an unbalanced stream.

The editor, not the profile, remembers what the viewer set, and it answers a frame later: a widget
starts from the profile's own value and reports the edited one on every frame after, and a button
answers true on the frame after it is pressed.

Writes roll back on every frame but the one that first carries a click or an edit and writes
something. That is what lets a profile keep panel state in its own vars, which the other hooks then
read off `src`, without an idle profile growing the heap sixty times a second. An interaction the
profile drops keeps nothing, so a button it ignores costs no work.

A frame that keeps its writes re-derives whatever it asked `demir_rebake` for, against the runtime
the full bake already initialized; `New()` does not run again, so the state survives. A
frame that asks for nothing re-derives nothing, so a profile that writes state without calling it
draws a panel whose switches never take effect.

`kinds` is a combinable `DEMIR_BAKE_APPEARANCE`, `_LIGHT` and `_HIGHLIGHT`. `groups` selects the
placements whose type `demir_define_group` put in one of them, and zero, or omitted, means all of
them. Several calls in a frame add up, and a request held back while a slider is being dragged
merges into the next. Naming a group is what keeps a panel responsive: on tgstation's Deltastation,
re-deriving appearances for every one of 182,598 placements takes 2.6s, where the disposal pipes
take 42ms and the cables 156ms.

An option whose reach is not a type subtree simply names no group and pays for the whole map, which
is what tgstation's smoothing toggle does: `smoothing_flags` is declared at a hundred-odd scattered
types, so there is no honest set to register.

Relighting first clears the neutral schema on the objects it covers, so a source the panel switched
off falls back to the values its own type declares instead of keeping the ones it was handed before.

`imgui_radio` answers like `imgui_button`, so a group is a run of them over one stored value:

```dm
if(imgui_radio("Wide", src.shape == SHAPE_WIDE))
	src.shape = SHAPE_WIDE
```

Every other widget answers with its value, so the profile, which is holding the old one anyway,
compares to find out what the viewer changed and what that costs:

```dm
var/cables = imgui_checkbox("Cables", src.show_cables)
if(cables != src.show_cables)
	src.show_cables = cables
	demir_rebake(DEMIR_BAKE_APPEARANCE, DEMIR_GROUP_CABLES)
```

`examples/profiles/tgstation.dm` keeps its options in vars on `/datum/demir/tgstation`, which `bake`
reads off `src` and `demir_apply_light` reaches through `demir_profile()`, to turn smoothing and
lighting off from the panel. Its
under-floor checkboxes are the reason the hook can reach appearances at all: tgstation covers
cables, pipes and disposals with a floor tile through `/datum/element/undertile`, which reacts to
`COMSIG_OBJ_HIDE` from `levelupdate()`. No components or signals run in a bake, so nothing ever
hides them and every network floats over the tiles. The profile reads the turf's own
`underfloor_accessibility` instead and reproduces the result, with a checkbox per network and one
more for half alpha. Atmos pipes are gated on `hide`, since `setup_hiding()` skips the `/visible`
subtypes that set it false, and only a network not already on `FLOOR_PLANE` is sunk onto it, so a
cable keeps the per-layer offsets it orders itself by.

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

A full bake has seven ordered stages:

1. **Instantiate** creates objects, applies constant map overrides, and links each map cell.
2. **Initialize** allocates the profile and runs its `New()` once, after the complete runtime
   world is linked.
3. **Prepare** calls `prepare` once per runtime object and commits successful setup for use by
   neighboring previews.
4. **Connections** calls `connections` once per runtime object and indexes matching channels.
5. **Highlights** calls `highlights` for each placement and records the tiles it claims.
6. **Light** calls `light` once per runtime object, harvests the neutral schema for every
   placement, and solves each z level's shared lighting corners.
7. **Smooth** calls `bake` for each placement and exports appearance changes.

`ui` runs outside those stages, once per editor frame, and `Bake::rebake` re-runs stages 5
through 7 when one of its frames keeps its writes.

The profile is allocated before any transaction is open, because object IDs are arena indices and a
rollback truncates: an ID minted inside a transaction and held outside the heap would dangle. Its
`New()` therefore keeps its writes unconditionally. A fault there is reported once and stops the
remaining stages, leaving every atom on its static appearance. Preparation commits when it succeeds;
a per-object fault excludes that object from later hooks.

`bake`, `highlights` and `connections` run in rolled-back transactions, so none of them can
accumulate state on the profile; the appearance cache depends on that. `ui` is the one hook whose
writes to `src` can survive, on the frames described above.
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
the prepare, connection, and light hooks to new objects, and rebakes the surrounding 3 by 3 by 3
neighborhood. Connection endpoint entries are replaced or removed with their placements, and the
channel index is rebuilt without rerunning hooks for unchanged objects. The update reuses the runtime
initialized by the full bake and never reruns `New()`. The vertical extent is needed by
codebases with pipes or other structures that connect between z levels. Incremental edits bypass
full-load cache reuse through the epoch, avoiding results derived from stale runtime globals. The
update reports only the edited instances and the neighbors whose composed appearance actually changed.

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

When a profile registers at least one node group, the editor adds a **Node** toolbar button and an
`N` shortcut. Double-click a registered object to open its cardinally connected component on the
active Z level. Handles appear on isolated objects, endpoints, elbows, and junctions; double-clicking
any object creates a handle there, and a newly dragged endpoint remains a handle. Double-click
picking uses the rendered visibility buffer, so eligible objects from different pipe layers on one
tile are selected from their visible pixels. If no sprite is hit, or a floor or area wins the visible
pick, the editor searches that tile for an eligible node object. Drag a handle to lay the shortest route around
registered blockers. Coordinate loops are removed before placement. The
editor copies the seeded object's exact type and map variables, reuses registered objects already on
the route, updates the map while dragging, and records the gesture as one undo step. Escape or
releasing an unreachable route restores every touched tile to its exact pre-drag contents. Hover a
connection line or one of its visible routed objects and right-click
to remove its registered group objects as one undoable edit. Each endpoint remains when it has
another connection and is removed when deleting the selected connection would leave it standalone;
this also makes a connection between two adjacent terminal nodes removable. Right-clicking an
already standalone handle removes all registered group objects on its tile. Unrelated objects remain,
and deletion is rejected as a whole when an affected tile is outside the current focus.

`editor::bake` translates placed prefabs into `vm::bake::Atom`s, keyed by `PrefabInstanceId`. Each
open map owns its bake. A new environment, a newly opened map, or a change in the number of z levels
bakes the whole map again. An edit, undo, or redo goes through `Bake::update`, and the sprites of
every placement it reports are rebuilt. Hiding a type rebuilds sprites from the cached bake without
running any DM.

Frame building applies fields changed by baking over the compatibility tree's static appearance.
The baker exports `icon` and `icon_state` together when either changes, so a runtime state is never
combined with an incompatible mapping-only icon sheet. An untouched atom still keeps its mapping-only
appearance. A type that exists only under a compatibility define remains drawable but is omitted from
the runtime world. Type IDs never cross between the two views; placed prefabs are resolved by path in each tree.
Overlay and underlay deltas become extra sprites owned by the placement, drawn in list order around
it. They inherit the owner's icon, dir, offsets, and floating layer and plane. Color and alpha
multiply with the owner's unless the overlay sets `RESET_COLOR` or `RESET_ALPHA`.

When a profile defines `light`, the map canvas always applies its baked lighting. The lighting
pass runs after map sprites and before area outlines, selection feedback, and placement previews, so
editor feedback and previews remain readable. Lighting has its own revision and GPU update range;
ordinary appearance edits do not re-upload the lightmap.

With the select tool active, **Selection guide lines** draws every baked connection incident to the
selected placement in the same orange guide style used for pixel offsets. Either endpoint can be
selected. Connections to another z level are projected onto the current level and marked with a
`Z<n>` badge. Every placement at the far end of a guide is also tinted orange in the map, in both
highlight styles, so a button shows which shutters it drives. Turning the setting off hides both
offset and connection guides along with the tint.

Highlights draw independently of that setting, in the color each descriptor chose: a wash over every
tile it covers and a marching dashed border along the sides that face outward, on the block
selection's stripe rhythm but leaving gaps rather than alternating with white, so a permanently
shaded region stays quiet. A `DEMIR_HIGHLIGHT_ALWAYS` highlight is drawn whenever its level is open, a
`_SELECTED` one only for the selected placement, and a `_HOVERED` one for anything under the cursor,
which trails the cursor by a frame. Highlights on another z level are not drawn, and tiles outside
the viewport are skipped.

Palette thumbnails resolve statically. An atom whose static `icon_state` is missing from its sheet,
which is how smoothed walls are declared, is baked alone in a one cell world at load, and the
derived appearance is used for its thumbnail. The placement preview still resolves statically.

A profile that defines `ui` draws into the editor's own dockspace, after the map views and
before the modal dialogs, so its windows dock and float like the editor's. The editor holds the
widget values, the pressed buttons and the collapsed nodes between frames and forgets the ones the
profile stopped drawing. A frame that keeps its writes queues a rebake, which is held until no
widget is being dragged, so working a slider costs one rebake on release rather than one per frame.

The settings tab lists grouped bake faults. It refreshes after every full bake.

## Sandbox and limits

Every call receives an instruction budget, call-depth limit, allocation budget, and maximum text
size. The default instruction budget is 100,000 operations, call depth is 48, allocations are capped
at 100,000 units, and an individual text result is capped at 1 MiB. Randomness is seeded from the
type and coordinates of the atom a hook was called about, not of its `src`, so rebuilding or removing
and restoring the same atom is repeatable. The same atom is what memo safety is measured against, so
a hook reading its own neighbourhood still caches.

A `ui` frame is bounded as well: 4096 commands, 16 nested windows and nodes, 128 characters
of label and 1024 of text. An `imgui_*` proc called outside the hook is blocked like any other
unsupported operation, and the editor logs one fault per distinct kind rather than one per frame.

File access, native libraries, networking, sleep, spawn, timers, and interactive input remain
blocked. DM `catch` can handle DM throws but cannot swallow sandbox or resource-limit faults. World
time and tick usage are deterministic, and no client or subsystem loop runs.

Profiles may write debug messages with `world.log << value`. The compiler driver and viewer forward
each line to stderr with a `DM:` prefix. The editor sends it through its normal logger, which writes
to stderr where available and to `latest.log` on Windows. Other output targets remain blocked.

## Validation

The VM tests cover neighborhood smoothing, incremental remove/restore, deterministic random results,
instance-variable cache separation, list-backed neighbor overlays, rolled-back connection metadata,
bounded recursive appearance export, clipped and edge-tagged highlights restored across edits,
corner lighting,
blockers, ambient/fullbright cells, incremental lightmap restoration, panel state held in a global
across committed and rolled-back frames, a rebake group narrowing a pass to the placements carrying
it, and balanced command streams from a profile that left a
window or node open. The compiler-driver check exercises preprocessing, semantic
analysis, bytecode generation, map translation, the seven bake stages, summary output, and incremental
restoration:

```sh
cargo test --workspace
cargo run --release --bin rmdc -- bake examples/env/test.dme examples/env/test.dmm --summary --check-edit
```

The bake summary reports how many placements carry connections and how many incident connections
they hold, plus how many placements are highlighted and how many tiles they cover, which is the
quickest way to tell whether a profile's `connections` or `highlights` is reaching the
codebase at all. A profile only takes effect through the copy the `.dme` includes, so a profile
edited in `examples/profiles` reports zero until it is copied into the codebase.

The editor tests cover whole-map baking without changing map bytes, overlay sprites, incremental
sprites matching a full rebuild, undo and redo through the bake, movable smoothing, standalone
thumbnails, selection guides and profile highlights across z levels, hiding a type without rebaking,
replaying a command stream including an unbalanced one, forgetting a widget the profile stopped
drawing, and that only a codebase
with a profile bakes.
