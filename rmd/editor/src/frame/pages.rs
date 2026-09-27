use std::collections::BTreeMap;

use render::{SpriteInstance, SpriteTexture, UpdateRange};

use super::{FrameInstances, RenderedPrefab, SpriteKey};
use crate::document::PrefabInstanceId;

const PAGE_CAPACITY: usize = 256;
/// number of sprites to fill into a page during load, left is spare to make tweaks faster
/// see `lay_out` below
const PAGE_FILL: usize = 192;
const SPARE_OWNER: PrefabInstanceId = match PrefabInstanceId::from_raw(u64::MAX) {
    Some(id) => id,
    None => unreachable!(),
};

fn spare_sprite(z: u32) -> SpriteInstance {
    SpriteInstance {
        owner: SPARE_OWNER,
        area_owner: None,
        texture: SpriteTexture::default(),
        x: -1.0e9,
        y: -1.0e9,
        width: 0.0,
        height: 0.0,
        z,
        is_area: false,
        area_edges: 0,
        lighting: render::SpriteLighting::Normal,
        color: [0.0; 4],
        depth: 0.0,
        transform: [0.0; 4],
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SpritePage {
    start: usize,
    len: usize,
    /// smallest key the page takes, It ends where the next page's begins
    lower: SpriteKey,
}

impl FrameInstances {
    pub fn live_sprites(&self) -> impl Iterator<Item = &SpriteInstance> {
        self.sprites.iter().filter(|sprite| sprite.owner != SPARE_OWNER)
    }

    pub(super) fn lay_out(&mut self, keyed: Vec<(SpriteKey, SpriteInstance)>, levels: u32) {
        self.pages.clear();
        let first = keyed.first().map_or(1, |(key, _)| key.0).min(1);
        let last = keyed.last().map_or(levels, |(key, _)| key.0).max(levels);
        let level = |z: u32| {
            let start = keyed.partition_point(|(key, _)| key.0 < z);
            let end = keyed.partition_point(|(key, _)| key.0 <= z);

            &keyed[start..end]
        };

        // give every level a page
        let slots = (first..=last)
            .map(|z| level(z).len().div_ceil(PAGE_FILL).max(1) * PAGE_CAPACITY)
            .sum::<usize>();

        let mut keys = Vec::with_capacity(slots);
        let mut sprites = Vec::with_capacity(slots);

        // for each leve...
        for z in first..=last {
            let level = level(z);
            let mut chunks = level.chunks(PAGE_FILL).collect::<Vec<_>>();
            if chunks.is_empty() {
                chunks.push(&[]);
            }

            // for each chunk...
            for (index, chunk) in chunks.into_iter().enumerate() {
                let lower = match chunk.first() {
                    Some((key, _)) if index > 0 => *key,
                    _ => (z, i32::MIN, i32::MIN, 0, 0),
                };

                let start_offset = sprites.len();

                // for each sprite...
                for (key, sprite) in chunk {
                    keys.push(*key);
                    sprites.push(*sprite);
                }

                let spare_key = keys.last().copied().filter(|_| !chunk.is_empty()).unwrap_or(lower);
                keys.resize(start_offset + PAGE_CAPACITY, spare_key);
                sprites.resize(start_offset + PAGE_CAPACITY, spare_sprite(z));
                self.pages.push(SpritePage {
                    start: start_offset,
                    len: chunk.len(),
                    lower,
                });
            }
        }

        self.sprite_sort_keys = keys;
        self.sprites = sprites;
    }

    fn page_of(&self, key: &SpriteKey) -> Option<usize> {
        let index = self.pages.partition_point(|page| page.lower <= *key).checked_sub(1)?;

        (self.pages[index].lower.0 == key.0).then_some(index)
    }

    fn remove_from_page(&mut self, page: usize, key: &SpriteKey) {
        let SpritePage { start, len, lower } = self.pages[page];
        let Ok(index) = self.sprite_sort_keys[start..start + len].binary_search(key) else {
            return;
        };

        let end = start + len;
        self.sprites.copy_within(start + index + 1..end, start + index);
        self.sprite_sort_keys.copy_within(start + index + 1..end, start + index);
        self.sprites[end - 1] = spare_sprite(lower.0);
        self.pages[page].len -= 1;
    }

    fn try_insert_into_page(&mut self, page: usize, key: SpriteKey, sprite: SpriteInstance) -> bool {
        let SpritePage { start, len, .. } = self.pages[page];
        if len == PAGE_CAPACITY {
            return false; // gg
        }

        let index = match self.sprite_sort_keys[start..start + len].binary_search(&key) {
            Ok(index) | Err(index) => index,
        };

        let end = start + len;
        self.sprites.copy_within(start + index..end, start + index + 1);
        self.sprite_sort_keys.copy_within(start + index..end, start + index + 1);
        self.sprites[start + index] = sprite;
        self.sprite_sort_keys[start + index] = key;
        self.pages[page].len += 1;

        true
    }

    fn keyed_live_sprites(&self) -> Vec<(SpriteKey, SpriteInstance)> {
        self.pages
            .iter()
            .flat_map(|page| page.start..page.start + page.len)
            .map(|index| (self.sprite_sort_keys[index], self.sprites[index]))
            .collect()
    }
}

pub(super) fn replace_owner_sprites(
    instances: &mut FrameInstances, rendered: &[(PrefabInstanceId, Option<RenderedPrefab>)],
) -> Vec<UpdateRange> {
    // each touched page as it was, to report only the ones that really changed
    let mut before = BTreeMap::<usize, Vec<SpriteInstance>>::new();
    let mut touch = |instances: &FrameInstances, page: usize| {
        before.entry(page).or_insert_with(|| {
            let start = instances.pages[page].start;
            instances.sprites[start..start + PAGE_CAPACITY].to_vec()
        });
    };

    for (owner, rendered) in rendered {
        if let Some(keys) = instances.owner_keys.remove(owner) {
            for key in &keys {
                if let Some(page) = instances.page_of(key) {
                    touch(instances, page);
                    instances.remove_from_page(page, key);
                }
            }
        }

        if let Some(primary) = rendered.as_ref().and_then(|rendered| rendered.primary) {
            instances.primary_sprites.insert(*owner, primary);
        } else {
            instances.primary_sprites.remove(owner);
        }

        if let Some(rendered) = rendered
            && !rendered.sprites.is_empty()
        {
            let keys = rendered.sprites.iter().map(|(key, _)| *key).collect();
            instances.owner_keys.insert(*owner, keys);
        }
    }

    let mut replacements = rendered
        .iter()
        .filter_map(|(_, rendered)| rendered.as_ref())
        .flat_map(|rendered| rendered.sprites.iter().copied())
        .peekable();

    while let Some((key, sprite)) = replacements.peek().copied() {
        let placed = instances.page_of(&key).is_some_and(|page| {
            touch(instances, page);
            instances.try_insert_into_page(page, key, sprite)
        });

        if !placed {
            // alloc failed

            let mut keyed = instances.keyed_live_sprites();
            keyed.extend(replacements);
            keyed.sort_by_key(|(key, _)| *key);
            let levels = instances.pages.last().map_or(1, |page| page.lower.0);
            instances.lay_out(keyed, levels);

            return vec![UpdateRange {
                start: 0,
                end: instances.sprites.len(),
            }];
        }

        replacements.next();
    }

    let mut ranges = Vec::<UpdateRange>::new();
    for (page, previous) in before {
        let start = instances.pages[page].start;
        let end = start + PAGE_CAPACITY;
        if instances.sprites[start..end] == previous[..] {
            continue;
        }

        match ranges.last_mut() {
            Some(range) if range.end == start => range.end = end,
            _ => ranges.push(UpdateRange { start, end }),
        }
    }

    ranges
}

#[cfg(test)]
pub(super) mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Map, Prefab, Size};

    use super::{PAGE_CAPACITY, SPARE_OWNER, spare_sprite};
    use crate::{
        command::Edit,
        document::{MapDocument, PrefabInstanceId},
        frame::{
            FrameInstances,
            PrefabUpdate,
            build,
            tests::{assert_render_data_matches, document, icons, textures, tree},
            update_prefab,
            update_prefabs,
        },
    };

    /// Pages sit back to back, hold sorted keys within their bounds, and pad with spare slots.
    pub(in crate::frame) fn assert_pages_hold(instances: &FrameInstances) {
        assert_eq!(instances.sprites.len(), instances.pages.len() * PAGE_CAPACITY);
        let mut live = 0;
        for (index, page) in instances.pages.iter().enumerate() {
            assert_eq!(page.start, index * PAGE_CAPACITY);
            assert!(page.len <= PAGE_CAPACITY);
            let next = instances.pages.get(index + 1).map(|next| next.lower);
            let keys = &instances.sprite_sort_keys[page.start..page.start + page.len];
            assert!(keys.windows(2).all(|keys| keys[0] < keys[1]));
            assert!(
                keys.iter()
                    .all(|key| *key >= page.lower && next.is_none_or(|next| *key < next))
            );

            let slots = &instances.sprites[page.start..page.start + PAGE_CAPACITY];
            let (used, spare) = slots.split_at(page.len);
            assert!(used.iter().all(|sprite| sprite.owner != SPARE_OWNER));
            assert!(spare.iter().all(|sprite| *sprite == spare_sprite(page.lower.0)));
            live += page.len;
        }

        let owned = instances.owner_keys.values().map(Vec::len).sum::<usize>();
        assert_eq!(owned, live);
        for (owner, keys) in &instances.owner_keys {
            for key in keys {
                let page = instances.pages[instances.page_of(key).expect("a page for every key")];
                let index = instances.sprite_sort_keys[page.start..page.start + page.len]
                    .binary_search(key)
                    .expect("an owner key is in its page");
                assert_eq!(instances.sprites[page.start + index].owner, *owner);
            }
        }
    }

    fn floor_map(width: u32, height: u32, levels: u32) -> Map {
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: levels,
        });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
        for level in &mut map.grid {
            for row in level {
                row.fill(key);
            }
        }

        map
    }

    fn place(document: &mut MapDocument, coord: Coord, path: &str) -> PrefabInstanceId {
        let placed = document.instantiate(Prefab::new(TreePath::parse(path)));
        let id = placed.id();
        let mut after = document.placed_tile(coord).unwrap();
        after.push(placed);
        let mut edit = Edit::new("place");
        edit.change(document, coord, after);
        assert!(document.apply(edit));

        id
    }

    #[test]
    fn a_placement_uploads_only_the_pages_it_touches() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);
        let mut document = document(floor_map(24, 24, 2));
        let mut instances = build(&tree, &icons, &textures, &document, 32);
        let slots = instances.sprites.len();

        assert_eq!(instances.pages.len(), 6);
        assert_pages_hold(&instances);

        let table = place(&mut document, Coord::new(5, 5, 1), "/obj/table");
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, table, 32);
        let PrefabUpdate::Buffers { sprites, .. } = update else {
            panic!("a placement must stay incremental");
        };

        assert_eq!(sprites.len(), 1);
        assert_eq!(sprites[0].end - sprites[0].start, PAGE_CAPACITY);
        assert_eq!(instances.sprites.len(), slots);
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn pages_follow_placements_overflow_removals_and_undo() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);
        let mut document = document(floor_map(24, 24, 2));
        let mut instances = build(&tree, &icons, &textures, &document, 32);

        // every table sorts into the same page, so this runs it out of spare slots
        for x in 1..=24 {
            for y in 1..=5 {
                let table = place(&mut document, Coord::new(x, y, 1), "/obj/table");
                update_prefab(&mut instances, &tree, &icons, &textures, &document, table, 32);
            }
        }

        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

        for x in 1..=24 {
            let coord = Coord::new(x, 3, 2);
            let owners = document.instance_ids_at(coord).to_vec();
            let mut edit = Edit::new("clear");
            edit.change(&document, coord, Vec::new());
            assert!(document.apply(edit));
            update_prefabs(&mut instances, &tree, &icons, &textures, &document, &owners, 32);
        }

        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

        for _ in 0..30 {
            let affected = document.undo_with_affected().expect("undo");
            update_prefabs(&mut instances, &tree, &icons, &textures, &document, &affected, 32);
        }

        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }
}
