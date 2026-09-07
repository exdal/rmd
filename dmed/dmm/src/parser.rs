use core::{
    location::Position,
    path::{PathFlags, TreePath},
    types::{Identifier, ListEntry, Value},
};
use std::{collections::HashMap, path::Path};

use crate::{
    Coord,
    Map,
    MapFormat,
    Prefab,
    Size,
    Tile,
    error::{MapError, MapErrorKind},
    key::Key,
};

pub type MapResult<T> = Result<T, MapError>;

/// `(x,y,z) = {" ... "}`
struct Block {
    origin: Coord,
    position: Position,
    /// `rows[0]` is the highest DM `y`
    rows: Vec<Vec<Key>>,
}

/// `"aa" = (/turf/open/floor/plating,/area/space)`
pub struct MapParser<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    line_offset: usize,
    key_length: usize,
    errors: Vec<MapError>,
}

// my god what an abomination, surely there is some genius idea behind why this map format is
// structured like this

impl<'a> MapParser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 0,
            line_offset: 0,
            key_length: 0,
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[MapError] { &self.errors }

    pub fn take_errors(&mut self) -> Vec<MapError> { std::mem::take(&mut self.errors) }

    pub fn position(&self) -> Position {
        Position::new(self.line + 1, self.offset.saturating_sub(self.line_offset) + 1)
    }

    fn error(&self, kind: MapErrorKind) -> MapError { MapError::new(kind, self.position()) }

    fn byte_at(&self, offset: usize) -> Option<u8> { self.source.as_bytes().get(offset).copied() }

    fn peek(&self) -> Option<u8> { self.byte_at(self.offset) }

    fn peek_at(&self, ahead: usize) -> Option<u8> { self.byte_at(self.offset.saturating_add(ahead)) }

    fn is_eof(&self) -> bool { self.offset >= self.source.len() }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset = self.offset.saturating_add(1);

        if byte == b'\n' {
            self.line = self.line.saturating_add(1);
            self.line_offset = self.offset;
        }

        Some(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.bump();
            return true;
        }

        false
    }

    fn expect(&mut self, byte: u8) -> MapResult<()> {
        if self.eat(byte) {
            return Ok(());
        }

        Err(self.error(MapErrorKind::Expected(char::from(byte))))
    }

    fn slice_from(&self, start: usize) -> MapResult<&'a str> {
        self.source
            .get(start..self.offset)
            .ok_or_else(|| self.error(MapErrorKind::UnexpectedEof))
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.bump();
        }
    }

    fn skip_to_end_of_line(&mut self) {
        while !self.is_eof() && self.peek() != Some(b'\n') {
            self.bump();
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => {
                    self.bump();
                },
                Some(b'/') if self.peek_at(1) == Some(b'/') => self.skip_to_end_of_line(),
                _ => return,
            }
        }
    }

    fn recover(&mut self) {
        loop {
            self.skip_to_end_of_line();

            if self.bump().is_none() {
                return;
            }

            if matches!(self.peek(), None | Some(b'"') | Some(b'(')) {
                return;
            }
        }
    }

    pub fn parse(&mut self) -> (Map, Vec<MapError>) {
        self.skip_trivia();
        let format = self.detect_format();
        let dictionary = self.parse_dictionary();
        let blocks = self.parse_blocks(&dictionary);
        let mut map = self.assemble(blocks);

        map.format = format;
        map.key_length = self.key_length.max(1);
        map.dictionary = dictionary;

        (std::mem::take(&mut map), self.take_errors())
    }

    /// `"aa" = (\n/turf/open/floor/plating,\n/area)`
    fn detect_format(&self) -> MapFormat {
        let Some(rest) = self.source.get(self.offset..) else {
            return MapFormat::Standard;
        };

        let Some(open) = rest.find("= (") else {
            return MapFormat::Standard;
        };

        match rest.as_bytes().get(open + 3) {
            Some(b'\n') => MapFormat::Tgm,
            _ => MapFormat::Standard,
        }
    }

    fn parse_dictionary(&mut self) -> HashMap<Key, Tile> {
        let mut dictionary = HashMap::new();

        loop {
            self.skip_trivia();
            if self.peek() != Some(b'"') {
                break;
            }

            match self.parse_entry() {
                Ok((key, text, tile)) => {
                    if dictionary.insert(key, tile).is_some() {
                        let error = self.error(MapErrorKind::DuplicateKey(text));
                        self.errors.push(error);
                    }
                },
                Err(error) => {
                    self.errors.push(error);
                    self.recover();
                },
            }
        }

        if dictionary.is_empty() {
            let error = self.error(MapErrorKind::ExpectedDictionary);
            self.errors.push(error);
        }

        dictionary
    }

    fn parse_entry(&mut self) -> MapResult<(Key, String, Tile)> {
        self.expect(b'"')?;
        let start = self.offset;
        while !matches!(self.peek(), None | Some(b'"' | b'\n')) {
            self.bump();
        }

        let text = self.slice_from(start)?.to_string();
        self.expect(b'"')?;

        // First key sets the width
        if self.key_length == 0 {
            self.key_length = text.len();
        } else if text.len() != self.key_length {
            return Err(self.error(MapErrorKind::InconsistentKeyLength {
                expected: self.key_length,
                found: text.len(),
            }));
        }

        let key = Key::parse(&text).map_err(|_| self.error(MapErrorKind::InvalidKey(text.clone())))?;

        self.skip_trivia();
        self.expect(b'=')?;
        self.skip_trivia();
        self.expect(b'(')?;

        let mut tile = Tile::new();
        loop {
            self.skip_trivia();
            tile.push(self.parse_prefab()?);
            self.skip_trivia();
            if self.eat(b',') {
                continue;
            }

            self.expect(b')')?;
            break;
        }

        Ok((key, text, tile))
    }

    /// `/obj/item/sword{pixel_x = 4; name = "sharp sword"}`
    fn parse_prefab(&mut self) -> MapResult<Prefab> {
        let start = self.offset;
        while matches!(self.peek(), Some(b'/' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
            self.bump();
        }

        let text = self.slice_from(start)?;
        if text.is_empty() || !text.starts_with('/') {
            return Err(self.error(MapErrorKind::MalformedPrefab(text.to_string())));
        }

        let path = TreePath::parse(text);
        // `/obj/proc/thing`
        if path.flags != PathFlags::IS_DATUM {
            return Err(self.error(MapErrorKind::MalformedPrefab(text.to_string())));
        }

        let mut prefab = Prefab::new(path);

        if !self.eat(b'{') {
            return Ok(prefab);
        }

        loop {
            self.skip_trivia();
            let name_start = self.offset;
            while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
                self.bump();
            }

            let name = self.slice_from(name_start)?;
            if name.is_empty() {
                return Err(self.error(MapErrorKind::MalformedPrefab(text.to_string())));
            }

            self.skip_trivia();
            self.expect(b'=')?;
            self.skip_trivia();
            let value_start = self.offset;
            let value = self.parse_value()?;
            let source = self.slice_from(value_start)?;
            prefab.set_var_from_source(Identifier::from(name), value, source);

            self.skip_trivia();
            if self.eat(b';') {
                continue;
            }

            self.expect(b'}')?;
            break;
        }

        Ok(prefab)
    }

    /// `null`, `4`, `"text"`, `/obj/item`, `list(...)`
    fn parse_value(&mut self) -> MapResult<Value> {
        match self.peek() {
            Some(b'"') => Ok(Value::Text(self.scan_quoted(b'"')?.to_string())),
            Some(b'\'') => Ok(Value::Resource(self.scan_quoted(b'\'')?.to_string())),
            Some(b'/') => {
                let start = self.offset;
                while matches!(self.peek(), Some(b'/' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
                    self.bump();
                }

                Ok(Value::Path(TreePath::parse(self.slice_from(start)?)))
            },
            Some(b'-' | b'+' | b'.' | b'0'..=b'9') => self.parse_number(),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => {
                let start = self.offset;
                while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
                    self.bump();
                }

                let word = self.slice_from(start)?;

                match word {
                    "null" => Ok(Value::Null),
                    "list" if self.peek() == Some(b'(') => self.parse_list(),
                    // `list(ACCEPTING = "DONATIONS")`
                    _ => Ok(Value::Path(TreePath::parse(word))),
                }
            },
            _ => Err(self.error(MapErrorKind::MalformedValue(String::new()))),
        }
    }

    fn parse_number(&mut self) -> MapResult<Value> {
        let start = self.offset;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.bump();
        }

        while matches!(self.peek(), Some(b'.' | b'0'..=b'9')) {
            self.bump();
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.bump();

            if matches!(self.peek(), Some(b'-' | b'+')) {
                self.bump();
            }

            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump();
            }
        }

        let text = self.slice_from(start)?;
        text.parse::<f32>()
            .map(Value::Num)
            .map_err(|_| self.error(MapErrorKind::MalformedValue(text.to_string())))
    }

    fn parse_list(&mut self) -> MapResult<Value> {
        self.expect(b'(')?;
        let mut entries = Vec::new();

        self.skip_trivia();
        if self.eat(b')') {
            return Ok(Value::List(entries));
        }

        loop {
            self.skip_trivia();
            let key = self.parse_value()?;
            self.skip_trivia();
            let value = if self.eat(b'=') {
                self.skip_trivia();
                Some(self.parse_value()?)
            } else {
                None
            };
            entries.push(ListEntry { key, value });

            self.skip_trivia();
            if self.eat(b',') {
                continue;
            }

            self.expect(b')')?;
            break;
        }

        Ok(Value::List(entries))
    }

    /// `"say \"hi\""`
    fn scan_quoted(&mut self, quote: u8) -> MapResult<&'a str> {
        self.expect(quote)?;
        let start = self.offset;

        loop {
            match self.peek() {
                None => return Err(self.error(MapErrorKind::UnterminatedString)),
                Some(b'\\') => {
                    self.bump();
                    self.bump();
                },
                Some(byte) if byte == quote => {
                    let body = self.slice_from(start)?;
                    self.bump();

                    return Ok(body);
                },
                _ => {
                    self.bump();
                },
            }
        }
    }

    fn parse_blocks(&mut self, dictionary: &HashMap<Key, Tile>) -> Vec<Block> {
        let mut blocks = Vec::new();

        if self.key_length == 0 {
            return blocks;
        }

        loop {
            self.skip_trivia();
            if self.peek() != Some(b'(') {
                break;
            }

            match self.parse_block(dictionary) {
                Ok(block) => blocks.push(block),
                Err(error) => {
                    self.errors.push(error);
                    self.recover();
                },
            }
        }

        if !self.is_eof() || blocks.is_empty() {
            let error = self.error(MapErrorKind::ExpectedGrid);
            self.errors.push(error);
        }

        blocks
    }

    fn parse_block(&mut self, dictionary: &HashMap<Key, Tile>) -> MapResult<Block> {
        let position = self.position();

        self.expect(b'(')?;
        let x = self.parse_coordinate()?;
        self.expect(b',')?;
        let y = self.parse_coordinate()?;
        self.expect(b',')?;
        let z = self.parse_coordinate()?;
        self.expect(b')')?;

        self.skip_spaces();
        self.expect(b'=')?;
        self.skip_spaces();
        self.expect(b'{')?;
        self.expect(b'"')?;
        self.skip_spaces();
        self.expect(b'\n')?;

        // DM coordinates start at `1`
        if x == 0 || y == 0 || z == 0 {
            return Err(MapError::new(MapErrorKind::MalformedGridHeader, position));
        }

        let mut rows: Vec<Vec<Key>> = Vec::new();
        loop {
            if self.peek() == Some(b'"') && self.peek_at(1) == Some(b'}') {
                self.bump();
                self.bump();
                break;
            }

            let row_position = self.position();
            let start = self.offset;
            self.skip_to_end_of_line();
            let line = self.slice_from(start)?.trim_end_matches('\r').as_bytes();
            if self.bump().is_none() && line.is_empty() {
                return Err(MapError::new(MapErrorKind::UnexpectedEof, row_position));
            }

            if line.is_empty() || !line.len().is_multiple_of(self.key_length) {
                return Err(MapError::new(MapErrorKind::RaggedGrid, row_position));
            }

            let mut row = Vec::with_capacity(line.len() / self.key_length);
            for chunk in line.chunks(self.key_length) {
                let text = String::from_utf8_lossy(chunk).into_owned();
                let key = Key::parse_bytes(chunk)
                    .map_err(|_| MapError::new(MapErrorKind::InvalidKey(text.clone()), row_position))?;
                if !dictionary.contains_key(&key) {
                    return Err(MapError::new(MapErrorKind::UnknownKey(text), row_position));
                }

                row.push(key);
            }

            if rows.first().is_some_and(|first| first.len() != row.len()) {
                return Err(MapError::new(MapErrorKind::RaggedGrid, row_position));
            }

            rows.push(row);
        }

        Ok(Block {
            origin: Coord::new(x, y, z),
            position,
            rows,
        })
    }

    fn parse_coordinate(&mut self) -> MapResult<u32> {
        let start = self.offset;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.bump();
        }

        let text = self.slice_from(start)?;
        text.parse::<u32>()
            .map_err(|_| self.error(MapErrorKind::MalformedGridHeader))
    }

    fn assemble(&mut self, blocks: Vec<Block>) -> Map {
        let mut size = Size::default();
        for block in &blocks {
            let width = block.rows.first().map_or(0, |row| row.len()) as u32;
            let height = block.rows.len() as u32;

            size.x = size.x.max(block.origin.x.saturating_add(width).saturating_sub(1));
            size.y = size.y.max(block.origin.y.saturating_add(height).saturating_sub(1));
            size.z = size.z.max(block.origin.z);
        }

        let mut map = Map::new(size);
        let mut filled = vec![vec![vec![false; size.x as usize]; size.y as usize]; size.z as usize];

        for block in &blocks {
            let height = block.rows.len();
            let Some(level) = block.origin.z.checked_sub(1).map(|z| z as usize) else {
                continue;
            };

            for (i, row) in block.rows.iter().enumerate() {
                // `y0 + (n - 1 - i)`
                let Some(dm_y) = height
                    .checked_sub(1)
                    .and_then(|last| last.checked_sub(i))
                    .and_then(|up| (block.origin.y as usize).checked_add(up))
                else {
                    continue;
                };

                // `grid[z][y][x]`
                let Some(index) = (size.y as usize).checked_sub(dm_y) else {
                    continue;
                };

                for (j, key) in row.iter().enumerate() {
                    let Some(column) = (block.origin.x as usize).checked_sub(1).and_then(|x| x.checked_add(j)) else {
                        continue;
                    };

                    let seen = filled
                        .get_mut(level)
                        .and_then(|level| level.get_mut(index))
                        .and_then(|row| row.get_mut(column));
                    let slot = map
                        .grid
                        .get_mut(level)
                        .and_then(|level| level.get_mut(index))
                        .and_then(|row| row.get_mut(column));

                    let (Some(seen), Some(slot)) = (seen, slot) else {
                        self.errors
                            .push(MapError::new(MapErrorKind::MalformedGridHeader, block.position));
                        continue;
                    };

                    if *seen {
                        let coord = Coord::new(column as u32 + 1, dm_y as u32, block.origin.z);
                        self.errors
                            .push(MapError::new(MapErrorKind::OverlappingBlocks(coord), block.position));
                        continue;
                    }

                    *slot = *key;
                    *seen = true;
                }
            }
        }

        for (level, rows) in filled.iter().enumerate() {
            for (index, row) in rows.iter().enumerate() {
                for (column, seen) in row.iter().enumerate() {
                    if *seen {
                        continue;
                    }

                    let coord = Coord::new(
                        column as u32 + 1,
                        (size.y as usize).saturating_sub(index) as u32,
                        level as u32 + 1,
                    );
                    let error = self.error(MapErrorKind::IncompleteGrid(coord));
                    self.errors.push(error);

                    return map;
                }
            }
        }

        map
    }
}

pub fn parse(source: &str) -> (Map, Vec<MapError>) { MapParser::new(source).parse() }

pub fn parse_value(source: &str) -> MapResult<Value> {
    let mut parser = MapParser::new(source);
    parser.skip_trivia();
    let value = parser.parse_value()?;
    parser.skip_trivia();

    if parser.is_eof() {
        Ok(value)
    } else {
        Err(parser.error(MapErrorKind::MalformedValue(source.to_string())))
    }
}

pub fn load(path: impl AsRef<Path>) -> std::io::Result<(Map, Vec<MapError>)> {
    let source = std::fs::read_to_string(path)?;

    Ok(parse(&source))
}

#[cfg(test)]
mod tests {
    use core::types::{Identifier, Value};

    use crate::{
        Coord,
        MapFormat,
        VarValue,
        error::MapErrorKind,
        key::Key,
        parser::{parse, parse_value},
    };

    const STANDARD: &str = concat!(
        "\"a\" = (/turf/wall,/area)\n",
        "\"b\" = (/turf/floor,/area)\n",
        "\n",
        "(1,1,1) = {\"\n",
        "aab\n",
        "bba\n",
        "\"}\n",
    );

    const TGM: &str = concat!(
        "//MAP CONVERTED BY dmm2tgm.py THIS HEADER COMMENT PREVENTS RECONVERSION, DO NOT REMOVE\n",
        "\"a\" = (\n",
        "/turf/wall,\n",
        "/area)\n",
        "\"b\" = (\n",
        "/turf/floor,\n",
        "/area)\n",
        "\n",
        "(1,1,1) = {\"\n",
        "a\n",
        "b\n",
        "\"}\n",
        "(2,1,1) = {\"\n",
        "a\n",
        "b\n",
        "\"}\n",
        "(3,1,1) = {\"\n",
        "b\n",
        "a\n",
        "\"}\n",
    );

    fn prefab_vars(source: &str) -> Vec<(Identifier, VarValue)> {
        let (map, errors) = parse(source);
        assert!(errors.is_empty(), "{errors:?}");

        let tile = map.tile_at(Coord::new(1, 1, 1)).expect("tile");
        tile.first().expect("prefab").vars.clone()
    }

    #[test]
    fn reads_a_standard_dictionary_and_grid() {
        let (map, errors) = parse(STANDARD);
        assert!(errors.is_empty(), "{errors:?}");

        assert_eq!((map.size.x, map.size.y, map.size.z), (3, 2, 1));
        assert_eq!(map.dictionary.len(), 2);
        assert_eq!(map.key_length, 1);
        assert_eq!(map.format, MapFormat::Standard);
    }

    #[test]
    fn flips_y_between_file_order_and_dm_coordinates() {
        let (map, _) = parse(STANDARD);

        assert_eq!(map.key_at(Coord::new(1, 1, 1)), Key::parse("b").ok());
        assert_eq!(map.key_at(Coord::new(3, 1, 1)), Key::parse("a").ok());
        assert_eq!(map.key_at(Coord::new(1, 2, 1)), Key::parse("a").ok());
        assert_eq!(map.key_at(Coord::new(3, 2, 1)), Key::parse("b").ok());
    }

    #[test]
    fn reads_tgm_column_blocks_into_the_same_grid() {
        let (standard, _) = parse(STANDARD);
        let (tgm, errors) = parse(TGM);
        assert!(errors.is_empty(), "{errors:?}");

        assert_eq!(tgm.format, MapFormat::Tgm);
        assert_eq!(tgm.size, standard.size);
        assert_eq!(tgm.grid, standard.grid);
    }

    #[test]
    fn reads_prefab_vars_in_both_layouts() {
        let inline = prefab_vars(concat!(
            "\"a\" = (/obj/thing{dir = 4; name = \"x\"},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));
        let block = prefab_vars(concat!(
            "\"a\" = (\n",
            "/obj/thing{\n\tdir = 4;\n\tname = \"x\"\n\t},\n",
            "/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        assert_eq!(inline, block);
        assert_eq!(inline.len(), 2);
    }

    #[test]
    fn keeps_prefab_vars_in_file_order() {
        let vars = prefab_vars(concat!(
            "\"a\" = (/obj/thing{name = \"x\"; id_tag = \"y\"},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        let names: Vec<&str> = vars.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["name", "id_tag"]);
    }

    #[test]
    fn reads_the_literal_subset() {
        let vars = prefab_vars(concat!(
            "\"a\" = (/obj/t{n = null; i = -8; f = 1.5; e = 5e+006; s = \"hi\"; ",
            "r = 'a.dmi'; p = /obj/other; l = list(\"a\",\"b\"); m = list(\"k\"=\"v\"); ",
            "w = list(BARE=1); z = list()},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        let get = |name: &str| {
            vars.iter()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, v)| v.value.clone())
        };

        assert_eq!(get("n"), Some(Value::Null));
        assert_eq!(get("i"), Some(Value::Num(-8.0)));
        assert_eq!(get("f"), Some(Value::Num(1.5)));
        assert_eq!(get("e"), Some(Value::Num(5e6)));
        assert_eq!(get("s"), Some(Value::Text("hi".into())));
        assert_eq!(get("r"), Some(Value::Resource("a.dmi".into())));
        assert!(matches!(get("p"), Some(Value::Path(path)) if path.absolute && path.len() == 2));
        assert!(matches!(get("l"), Some(Value::List(entries)) if entries.len() == 2));
        assert!(matches!(get("m"), Some(Value::List(entries)) if entries[0].value.is_some()));
        assert!(matches!(get("z"), Some(Value::List(entries)) if entries.is_empty()));
        assert!(matches!(get("w"), Some(Value::List(entries))
            if matches!(&entries[0].key, Value::Path(path) if !path.absolute)));
    }

    #[test]
    fn parses_one_complete_variable_value() {
        let value = parse_value(" list(\"a\", key = /obj/item, nested = list(1, null)) ").unwrap();

        assert!(matches!(value, Value::List(entries) if entries.len() == 3));
        assert_eq!(
            parse_value("'icons/items.dmi'").unwrap(),
            Value::Resource("icons/items.dmi".into())
        );
        assert_eq!(parse_value("-1.5e2").unwrap(), Value::Num(-150.0));
    }

    #[test]
    fn standalone_value_parser_rejects_partial_or_malformed_input() {
        assert!(parse_value("1 trailing").is_err());
        assert!(parse_value("\"unterminated").is_err());
        assert!(parse_value("list(1,)").is_err());
        assert!(parse_value("").is_err());
    }

    #[test]
    fn keeps_the_source_spelling_only_when_it_is_unusual() {
        let vars = prefab_vars(concat!(
            "\"a\" = (/obj/t{plain = 4; exponent = 2e+005; trailing = 1.30; ",
            "spaced = list(\"a\", \"b\"); tight = list(\"a\",\"b\")},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        let get = |name: &str| {
            vars.iter()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, var)| var.clone())
        };

        assert_eq!(get("plain").and_then(|var| var.verbatim().map(str::to_string)), None);
        assert_eq!(get("tight").and_then(|var| var.verbatim().map(str::to_string)), None);

        assert_eq!(get("exponent").as_ref().and_then(VarValue::verbatim), Some("2e+005"));
        assert_eq!(get("trailing").as_ref().and_then(VarValue::verbatim), Some("1.30"));
        assert_eq!(
            get("spaced").as_ref().and_then(VarValue::verbatim),
            Some("list(\"a\", \"b\")")
        );

        assert_eq!(get("exponent").map(|var| var.value), Some(Value::Num(2e5)));
    }

    #[test]
    fn editing_a_var_drops_the_source_spelling() {
        let (map, _) = parse(concat!(
            "\"a\" = (/obj/t{charge = 2e+005},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        let mut prefab = map
            .tile_at(Coord::new(1, 1, 1))
            .and_then(|tile| tile.first())
            .cloned()
            .unwrap();
        assert!(prefab.vars[0].1.verbatim().is_some());

        prefab.set_var("charge".into(), Value::Num(2e5));
        assert!(prefab.vars[0].1.verbatim().is_none());
    }

    #[test]
    fn keeps_string_escapes_raw() {
        let vars = prefab_vars(concat!(
            "\"a\" = (/obj/t{n = \"say \\\"hi\\\"\"; d = \"an \\improper thing\"},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        assert_eq!(vars[0].1.value, Value::Text("say \\\"hi\\\"".into()));
        assert_eq!(vars[1].1.value, Value::Text("an \\improper thing".into()));
    }

    #[test]
    fn delimiters_inside_strings_do_not_terminate_a_var_list() {
        let vars = prefab_vars(concat!(
            "\"a\" = (/obj/t{n = \"x;y)z}\"; d = 1},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].1.value, Value::Text("x;y)z}".into()));
    }

    #[test]
    fn takes_the_key_width_from_the_file() {
        let (map, errors) = parse(concat!(
            "\"aa\" = (/turf/wall,/area)\n",
            "\"ab\" = (/turf/floor,/area)\n",
            "\n(1,1,1) = {\"\naaab\n\"}\n",
        ));

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(map.key_length, 2);
        assert_eq!(map.size.x, 2);
    }

    #[test]
    fn reads_multiple_z_levels() {
        let (map, errors) = parse(concat!(
            "\"a\" = (/turf/wall,/area)\n",
            "\"b\" = (/turf/floor,/area)\n",
            "\n(1,1,1) = {\"\nab\n\"}\n",
            "\n(1,1,2) = {\"\nba\n\"}\n",
        ));

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!((map.size.x, map.size.y, map.size.z), (2, 1, 2));
        assert_eq!(map.key_at(Coord::new(1, 1, 2)), Key::parse("b").ok());
    }

    #[test]
    fn detects_tgm_without_being_fooled_by_a_paren_in_a_string() {
        let (map, errors) = parse(concat!(
            "\"a\" = (/obj/t{n = \"a (\nb\"},/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(map.format, MapFormat::Standard);
    }

    #[test]
    fn recovers_from_a_bad_prefab_and_keeps_going() {
        let (map, errors) = parse(concat!(
            "\"a\" = (/turf/wall,/area)\n",
            "\"b\" = (not a path,/area)\n",
            "\"c\" = (/turf/floor,/area)\n",
            "\n(1,1,1) = {\"\nac\n\"}\n",
        ));

        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0].kind, MapErrorKind::MalformedPrefab(_)));
        assert_eq!(map.dictionary.len(), 2);
        assert_eq!(map.size.x, 2);
    }

    #[test]
    fn reports_an_unknown_grid_key_on_its_own_line() {
        let (_, errors) = parse(concat!("\"a\" = (/turf/wall,/area)\n", "\n(1,1,1) = {\"\naz\n\"}\n",));

        assert!(
            errors
                .iter()
                .any(|error| matches!(&error.kind, MapErrorKind::UnknownKey(key) if key == "z"))
        );
        assert_eq!(errors[0].position.line, 4);
    }

    #[test]
    fn reports_a_ragged_grid() {
        let (_, errors) = parse(concat!("\"a\" = (/turf/wall,/area)\n", "\n(1,1,1) = {\"\naa\na\n\"}\n",));

        assert!(
            errors
                .iter()
                .any(|error| matches!(error.kind, MapErrorKind::RaggedGrid))
        );
    }

    #[test]
    fn reports_a_key_of_the_wrong_width() {
        let (_, errors) = parse(concat!(
            "\"aa\" = (/turf/wall,/area)\n",
            "\"b\" = (/turf/floor,/area)\n",
            "\n(1,1,1) = {\"\naa\n\"}\n",
        ));

        assert!(
            errors
                .iter()
                .any(|error| matches!(error.kind, MapErrorKind::InconsistentKeyLength { .. }))
        );
    }

    #[test]
    fn reports_a_hole_left_by_a_missing_column() {
        let (_, errors) = parse(concat!(
            "\"a\" = (/turf/wall,/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
            "(3,1,1) = {\"\na\n\"}\n",
        ));

        assert!(
            errors
                .iter()
                .any(|error| matches!(error.kind, MapErrorKind::IncompleteGrid(_)))
        );
    }

    #[test]
    fn rejects_a_reserved_segment_in_a_prefab_path() {
        let (_, errors) = parse(concat!(
            "\"a\" = (/obj/proc/thing,/area)\n",
            "\n(1,1,1) = {\"\na\n\"}\n",
        ));

        assert!(
            errors
                .iter()
                .any(|error| matches!(error.kind, MapErrorKind::MalformedPrefab(_)))
        );
    }
}
