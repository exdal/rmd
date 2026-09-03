use core::{path::TreePath, types::Value};
use std::{fmt::Write, path::Path};

use crate::{Map, MapFormat, Prefab, Tile, VarValue, key::Key};

/// `dmm2tgm.py` reconversion guard
const TGM_HEADER: &str = "//MAP CONVERTED BY dmm2tgm.py THIS HEADER COMMENT PREVENTS RECONVERSION, DO NOT REMOVE\n";

pub struct MapWriter<'a> {
    map: &'a Map,
    format: MapFormat,
}

impl<'a> MapWriter<'a> {
    pub fn new(map: &'a Map) -> Self {
        Self {
            map,
            format: map.format,
        }
    }

    pub fn with_format(mut self, format: MapFormat) -> Self {
        self.format = format;

        self
    }

    pub fn write(&self) -> String { self.to_string() }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> { std::fs::write(path, self.write()) }

    fn write_dictionary(&self, out: &mut impl Write) -> std::fmt::Result {
        let mut entries: Vec<(&Key, &Tile)> = self.map.dictionary.iter().collect();
        entries.sort_unstable_by_key(|(key, _)| **key);

        for (key, tile) in entries {
            let mut text = String::new();
            key.write_to(&mut text, self.map.key_length);

            out.write_char('"')?;
            out.write_str(&text)?;
            out.write_str("\" = (")?;

            let separator = match self.format {
                MapFormat::Standard => ",",
                MapFormat::Tgm => ",\n",
            };

            if self.format == MapFormat::Tgm {
                out.write_char('\n')?;
            }

            for (index, prefab) in tile.iter().enumerate() {
                if index > 0 {
                    out.write_str(separator)?;
                }

                self.write_prefab(out, prefab)?;
            }

            out.write_str(")\n")?;
        }

        Ok(())
    }

    fn write_prefab(&self, out: &mut impl Write, prefab: &Prefab) -> std::fmt::Result {
        write_path(out, &prefab.path)?;

        if prefab.vars.is_empty() {
            return Ok(());
        }

        let (open, separator, close) = match self.format {
            MapFormat::Standard => ("{", "; ", "}"),
            MapFormat::Tgm => ("{\n\t", ";\n\t", "\n\t}"),
        };

        out.write_str(open)?;

        for (index, (name, var)) in prefab.vars.iter().enumerate() {
            if index > 0 {
                out.write_str(separator)?;
            }

            write!(out, "{name} = ")?;
            write_var(out, var)?;
        }

        out.write_str(close)?;

        Ok(())
    }

    fn write_grid(&self, out: &mut impl Write) -> std::fmt::Result {
        let mut key = String::new();

        for (level, rows) in self.map.grid.iter().enumerate() {
            // New grid section or z level
            out.write_char('\n')?;

            match self.format {
                MapFormat::Standard => {
                    writeln!(out, "(1,1,{}) = {{\"", level + 1)?;

                    for row in rows {
                        for cell in row {
                            key.clear();
                            cell.write_to(&mut key, self.map.key_length);
                            out.write_str(&key)?;
                        }

                        out.write_char('\n')?;
                    }

                    out.write_str("\"}\n")?;
                },
                MapFormat::Tgm => {
                    let width = rows.first().map_or(0, |row| row.len());

                    for column in 0..width {
                        writeln!(out, "({},1,{}) = {{\"", column + 1, level + 1)?;

                        for row in rows {
                            let Some(cell) = row.get(column) else {
                                continue;
                            };

                            key.clear();
                            cell.write_to(&mut key, self.map.key_length);
                            out.write_str(&key)?;
                            out.write_char('\n')?;
                        }

                        out.write_str("\"}\n")?;
                    }
                },
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for MapWriter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.format == MapFormat::Tgm {
            f.write_str(TGM_HEADER)?;
        }

        self.write_dictionary(f)?;

        self.write_grid(f)
    }
}

/// `list(ACCEPTING = "DONATIONS")`
fn write_path(out: &mut impl Write, path: &TreePath) -> std::fmt::Result {
    for (index, segment) in path.segments.iter().enumerate() {
        if index > 0 || path.absolute {
            out.write_char('/')?;
        }

        out.write_str(segment.as_str())?;
    }

    Ok(())
}

fn write_var(out: &mut impl Write, var: &VarValue) -> std::fmt::Result {
    match var.verbatim() {
        Some(source) => out.write_str(source),
        None => write_value(out, &var.value),
    }
}

pub(crate) fn write_value(out: &mut impl Write, value: &Value) -> std::fmt::Result {
    match value {
        Value::Null | Value::Unevaluated => out.write_str("null"),
        // `4`, not `4.0`
        Value::Num(number) => write!(out, "{number}"),
        // `"an \improper thing"`
        Value::Text(text) => write!(out, "\"{text}\""),
        Value::Resource(text) => write!(out, "'{text}'"),
        Value::Path(path) => write_path(out, path),
        Value::List(entries) => {
            out.write_str("list(")?;

            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    out.write_char(',')?;
                }

                write_value(out, &entry.key)?;

                if let Some(value) = &entry.value {
                    out.write_char('=')?;
                    write_value(out, value)?;
                }
            }

            out.write_char(')')
        },
    }
}

pub fn write(map: &Map) -> String { MapWriter::new(map).write() }

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use crate::{
        Map,
        MapFormat,
        Prefab,
        Size,
        key::Key,
        parser::parse,
        writer::{MapWriter, write},
    };

    const STANDARD: &str = concat!(
        "\"a\" = (/turf/wall,/area)\n",
        "\"b\" = (/obj/thing{name = \"x\"; id_tag = \"y\"},/turf/floor,/area)\n",
        "\n",
        "(1,1,1) = {\"\n",
        "aab\n",
        "bba\n",
        "\"}\n",
    );

    const TGM: &str = concat!(
        "//MAP CONVERTED BY dmm2tgm.py THIS HEADER COMMENT PREVENTS RECONVERSION, DO NOT REMOVE\n",
        "\"aa\" = (\n",
        "/turf/wall,\n",
        "/area)\n",
        "\"ab\" = (\n",
        "/obj/thing{\n",
        "\tname = \"say \\\"hi\\\"\";\n",
        "\tid_tag = \"an \\improper tag\";\n",
        "\ticon = 'a.dmi';\n",
        "\tdir = -4;\n",
        "\tlayer = 2.5;\n",
        "\tcharge = 2e+005;\n",
        "\tdamage = 1.30;\n",
        "\tnetwork = list(\"ss13\", \"engine\");\n",
        "\ttags = list(\"a\",\"b\");\n",
        "\tmap = list(\"k\"=\"v\");\n",
        "\tbare = list(BARE=1)\n",
        "\t},\n",
        "/turf/floor,\n",
        "/area)\n",
        "\n",
        "(1,1,1) = {\"\n",
        "aa\n",
        "ab\n",
        "\"}\n",
        "(2,1,1) = {\"\n",
        "ab\n",
        "aa\n",
        "\"}\n",
        "\n",
        "(1,1,2) = {\"\n",
        "ab\n",
        "ab\n",
        "\"}\n",
        "(2,1,2) = {\"\n",
        "aa\n",
        "aa\n",
        "\"}\n",
    );

    fn round_trip(source: &str) {
        let (map, errors) = parse(source);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(write(&map), source);
    }

    #[test]
    fn round_trips_standard_bytes() { round_trip(STANDARD); }

    #[test]
    fn round_trips_tgm_bytes() { round_trip(TGM); }

    #[test]
    fn converts_between_the_two_formats_without_losing_anything() {
        let (standard, _) = parse(STANDARD);
        let as_tgm = MapWriter::new(&standard).with_format(MapFormat::Tgm).write();

        let (back, errors) = parse(&as_tgm);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(back.format, MapFormat::Tgm);
        assert_eq!(back.grid, standard.grid);
        assert_eq!(back.dictionary, standard.dictionary);
    }

    #[test]
    fn writes_the_dictionary_in_ascending_key_order() {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });

        for raw in ["ac", "aa", "ab"] {
            let key = Key::parse(raw).unwrap();
            map.dictionary.insert(key, vec![Prefab::new(TreePath::parse("/area"))]);
        }

        map.key_length = 2;

        let written = write(&map);
        let keys: Vec<&str> = written
            .lines()
            .filter(|line| line.starts_with('"') && line.contains("\" = ("))
            .filter_map(|line| line.get(1..3))
            .collect();

        assert_eq!(keys, ["aa", "ab", "ac"]);
    }

    #[test]
    fn writes_integral_numbers_without_a_decimal_point() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/t"));
        for (name, value) in [("a", 4.0), ("b", 1800.0), ("c", -10.0), ("d", 0.5)] {
            prefab.set_var(name.into(), Value::Num(value));
        }

        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        map.dictionary.insert(Key(0), vec![prefab]);

        assert!(write(&map).contains("{a = 4; b = 1800; c = -10; d = 0.5}"));
    }

    #[test]
    fn writes_a_bare_list_key_without_a_leading_slash() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/t"));
        prefab.set_var("k".into(), Value::Path(TreePath::parse("BARE")));

        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        map.dictionary.insert(Key(0), vec![prefab]);

        assert!(write(&map).contains("{k = BARE}"));
    }

    #[test]
    fn writes_keys_at_the_maps_width_not_the_minimum() {
        let (map, _) = parse(concat!("\"aa\" = (/turf/wall,/area)\n", "\n(1,1,1) = {\"\naa\n\"}\n",));

        assert_eq!(map.key_length, 2);
        assert!(write(&map).contains("\"aa\" = ("));
    }
}
