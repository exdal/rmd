use crate::error::MetadataError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    South,
    North,
    East,
    West,
    Southeast,
    Southwest,
    Northeast,
    Northwest,
}

impl Dir {
    pub const ORDER: [Dir; 8] = [
        Dir::South,
        Dir::North,
        Dir::East,
        Dir::West,
        Dir::Southeast,
        Dir::Southwest,
        Dir::Northeast,
        Dir::Northwest,
    ];

    pub fn to_bits(self) -> u32 {
        match self {
            Dir::North => 1,
            Dir::South => 2,
            Dir::East => 4,
            Dir::West => 8,
            Dir::Northeast => 5,
            Dir::Northwest => 9,
            Dir::Southeast => 6,
            Dir::Southwest => 10,
        }
    }

    pub fn from_bits(bits: u32) -> Option<Self> {
        Some(match bits {
            1 => Dir::North,
            2 => Dir::South,
            4 => Dir::East,
            8 => Dir::West,
            5 => Dir::Northeast,
            9 => Dir::Northwest,
            6 => Dir::Southeast,
            10 => Dir::Southwest,
            _ => return None,
        })
    }

    pub fn index_within(self, dirs: u32) -> usize {
        let index = Dir::ORDER.iter().position(|d| *d == self).unwrap_or(0);
        if index < dirs as usize { index } else { 0 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IconState {
    pub name: String,
    /// 1, 4 or 8
    pub dirs: u32,
    pub frames: u32,
    pub delays: Vec<f32>,
    pub rewind: bool,
    pub movement: bool,
    /// 0 loops forever
    pub loop_count: u32,
    pub hotspot: Option<(u32, u32, u32)>,
    pub offset: usize,
}

impl IconState {
    pub fn sprite_index(&self, dir: Dir, frame: u32) -> usize {
        let frame = frame.min(self.frames.saturating_sub(1));

        self.offset + (frame as usize * self.dirs as usize) + dir.index_within(self.dirs)
    }

    pub fn sprite_count(&self) -> usize { (self.dirs * self.frames) as usize }

    pub fn is_animated(&self) -> bool { self.frames > 1 }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub version: String,
    pub width: u32,
    pub height: u32,
    pub states: Vec<IconState>,
}

impl std::error::Error for MetadataError {}

impl Metadata {
    pub fn find(&self, name: &str) -> Option<&IconState> { self.states.iter().find(|state| state.name == name) }

    pub fn sprite_rect(&self, index: usize, sheet_width: u32) -> (u32, u32, u32, u32) {
        let columns = (sheet_width / self.width).max(1) as usize;
        let x = (index % columns) as u32 * self.width;
        let y = (index / columns) as u32 * self.height;

        (x, y, self.width, self.height)
    }

    /// `# BEGIN DMI ... # END DMI`
    pub fn parse_description(description: &str) -> Result<Self, MetadataError> {
        let mut metadata = Metadata {
            width: 32,
            height: 32,
            ..Default::default()
        };

        let mut offset = 0;
        let mut started = false;

        for line in description.lines() {
            let line = line.trim();
            if line == "# BEGIN DMI" {
                started = true;
                continue;
            }

            if line == "# END DMI" {
                break;
            }

            if !started || line.is_empty() {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                return Err(MetadataError(format!("expected `key = value`, got '{line}'")));
            };

            let key = key.trim();
            let value = value.trim();

            match key {
                "version" => metadata.version = value.to_string(),
                "width" => metadata.width = parse_u32(value)?,
                "height" => metadata.height = parse_u32(value)?,
                "state" => {
                    if let Some(previous) = metadata.states.last() {
                        offset += previous.sprite_count();
                    }

                    metadata.states.push(IconState {
                        name: value.trim_matches('"').to_string(),
                        dirs: 1,
                        frames: 1,
                        offset,
                        ..Default::default()
                    });
                },
                _ => {
                    let Some(state) = metadata.states.last_mut() else {
                        continue;
                    };

                    match key {
                        "dirs" => state.dirs = parse_u32(value)?,
                        "frames" => state.frames = parse_u32(value)?,
                        "delay" => {
                            state.delays = value
                                .split(',')
                                .map(|part| part.trim().parse::<f32>().unwrap_or(1.0))
                                .collect();
                        },
                        "rewind" => state.rewind = value != "0",
                        "movement" => state.movement = value != "0",
                        "loop" => state.loop_count = parse_u32(value)?,
                        "hotspot" => {
                            let parts: Vec<u32> = value.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                            if let [x, y, frame] = parts[..] {
                                state.hotspot = Some((x, y, frame));
                            }
                        },
                        _ => {},
                    }
                },
            }
        }

        Ok(metadata)
    }
}

fn parse_u32(value: &str) -> Result<u32, MetadataError> {
    value
        .parse::<f32>()
        .map(|v| v as u32)
        .map_err(|_| MetadataError(format!("expected a number, got '{value}'")))
}

#[cfg(test)]
mod tests {
    use crate::metadata::{Dir, Metadata};

    const DESCRIPTION: &str = "# BEGIN DMI\nversion = 4.0\n\twidth = 32\n\theight = 32\nstate = \"chair\"\n\tdirs = \
                               4\n\tframes = 1\nstate = \"table\"\n\tdirs = 1\n\tframes = 2\n\tdelay = 2,2\n# END \
                               DMI\n";

    #[test]
    fn parses_states() {
        let metadata = Metadata::parse_description(DESCRIPTION).unwrap();

        assert_eq!(metadata.width, 32);
        assert_eq!(metadata.states.len(), 2);

        let chair = metadata.find("chair").unwrap();
        assert_eq!(chair.dirs, 4);
        assert_eq!(chair.offset, 0);

        let table = metadata.find("table").unwrap();
        assert_eq!(table.offset, 4);
        assert_eq!(table.delays, vec![2.0, 2.0]);
    }

    #[test]
    fn indexes_sprites_frame_major() {
        let metadata = Metadata::parse_description(DESCRIPTION).unwrap();
        let chair = metadata.find("chair").unwrap();

        assert_eq!(chair.sprite_index(Dir::South, 0), 0);
        assert_eq!(chair.sprite_index(Dir::West, 0), 3);
        // `dirs = 1`
        assert_eq!(metadata.find("table").unwrap().sprite_index(Dir::West, 1), 5);
    }
}
