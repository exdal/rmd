use std::path::Path;

use super::{
    Haystack, Query,
    maps::{MapCandidate, rank_maps},
    score,
};

fn path_score(query: &str, path: &str) -> Option<u32> {
    score(&Query::parse(query), &Haystack::new(path, None, None))
}

#[test]
fn tokens_score_by_where_they_land_and_all_must_match() {
    assert!(
        path_score("airlock", "/obj/machinery/door/airlock")
            > path_score("airlock", "/obj/machinery/door/airlock/engineering")
    );
    assert_eq!(
        path_score("airlock eng", "/obj/machinery/door/airlock/engineering"),
        Some(60 + 100)
    );
    assert_eq!(
        path_score("airlock eng", "/obj/machinery/door/airlock/glass_engineering"),
        Some(60 + 80)
    );
    assert_eq!(path_score("airlock eng", "/obj/machinery/door/airlock"), None);
    assert_eq!(path_score("mchnry", "/obj/machinery"), Some(10));
}

#[test]
fn names_and_overrides_match_when_the_path_does_not() {
    let haystack = Haystack::new("/obj/item/foo", Some("Welding Tool"), Some("desc = \"Rusty\""));

    assert_eq!(score(&Query::parse("weld"), &haystack), Some(70));
    assert_eq!(score(&Query::parse("rusty"), &haystack), Some(30));
}

#[test]
fn the_map_query_does_not_match_the_codebase_directory_itself() {
    // The haystack is relative, so a codebase living under a directory called "maps" must not make
    // every one of its maps match the word.
    let base = Path::new("/home/maps/tg");
    let maps = [MapCandidate::new(Some(base), Path::new("/home/maps/tg/station.dmm"))];

    assert!(rank_maps(&maps, &Query::parse("home")).is_empty());
}

#[test]
fn the_real_map_ranks_before_ruins_that_mention_it() {
    let base = Path::new("/tg");
    let paths = [
        "_maps/RandomRuins/LavaRuins/lavaland_surface_gas.dmm",
        "_maps/safehouses/lavaland_boss.dmm",
        "_maps/minigame/mafia/mafia_lavaland.dmm",
        "_maps/RandomRuins/IceRuins/icemoon_underground_lavaland.dmm",
        "_maps/map_files/Mining/Lavaland.dmm",
    ];
    let maps = paths.map(|path| MapCandidate::new(Some(base), &base.join(path)));

    let ranked = rank_maps(&maps, &Query::parse("lavaland"));

    assert_eq!(ranked.first(), Some(&4));
}
