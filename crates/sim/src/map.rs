//! Integer map coordinates, matching Factorio's own representation.

/// Sub-tile precision used by Factorio for positions: 1/256 of a tile.
pub const SUBTILES_PER_TILE: i32 = 256;

/// A position on a surface in 1/256-tile units (Factorio's internal `MapPosition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MapPosition {
    pub x: i32,
    pub y: i32,
}

impl MapPosition {
    pub const fn from_tiles(x: i32, y: i32) -> Self {
        MapPosition { x: x * SUBTILES_PER_TILE, y: y * SUBTILES_PER_TILE }
    }

    pub const fn tile(self) -> TilePosition {
        TilePosition { x: self.x.div_euclid(SUBTILES_PER_TILE), y: self.y.div_euclid(SUBTILES_PER_TILE) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TilePosition {
    pub x: i32,
    pub y: i32,
}

/// Factorio 2.0 uses 16 directions; the 4 cardinal ones are multiples of 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Direction(pub u8);

impl Direction {
    pub const NORTH: Direction = Direction(0);
    pub const EAST: Direction = Direction(4);
    pub const SOUTH: Direction = Direction(8);
    pub const WEST: Direction = Direction(12);

    pub const fn rotate_cw(self) -> Direction {
        Direction((self.0 + 4) % 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_positions_floor_to_tiles() {
        assert_eq!(MapPosition { x: -1, y: 255 }.tile(), TilePosition { x: -1, y: 0 });
    }
}
