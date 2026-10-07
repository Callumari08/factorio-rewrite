//! Integer map coordinates, matching Factorio's own representation.

/// Sub-tile precision used by Factorio for positions: 1/256 of a tile.
pub const SUBTILES_PER_TILE: i32 = 256;

/// Tiles per chunk side.
pub const CHUNK_SIZE: i32 = 32;

/// A position on a surface in 1/256-tile units (Factorio's internal `MapPosition`).
/// `y` grows southwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MapPosition {
    pub x: i32,
    pub y: i32,
}

impl MapPosition {
    pub const fn new(x: i32, y: i32) -> Self {
        MapPosition { x, y }
    }

    pub const fn from_tiles(x: i32, y: i32) -> Self {
        MapPosition { x: x * SUBTILES_PER_TILE, y: y * SUBTILES_PER_TILE }
    }

    /// Centre of the given tile.
    pub const fn tile_center(t: TilePosition) -> Self {
        MapPosition {
            x: t.x * SUBTILES_PER_TILE + SUBTILES_PER_TILE / 2,
            y: t.y * SUBTILES_PER_TILE + SUBTILES_PER_TILE / 2,
        }
    }

    pub const fn tile(self) -> TilePosition {
        TilePosition { x: self.x.div_euclid(SUBTILES_PER_TILE), y: self.y.div_euclid(SUBTILES_PER_TILE) }
    }

    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        MapPosition { x: self.x + dx, y: self.y + dy }
    }

    /// Squared distance in subtiles², as i64 to avoid overflow.
    pub fn distance_sq(self, other: MapPosition) -> i64 {
        let dx = (self.x - other.x) as i64;
        let dy = (self.y - other.y) as i64;
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TilePosition {
    pub x: i32,
    pub y: i32,
}

impl TilePosition {
    pub const fn new(x: i32, y: i32) -> Self {
        TilePosition { x, y }
    }

    pub const fn chunk(self) -> ChunkPosition {
        ChunkPosition { x: self.x.div_euclid(CHUNK_SIZE), y: self.y.div_euclid(CHUNK_SIZE) }
    }

    pub const fn step(self, d: Direction) -> TilePosition {
        let [dx, dy] = d.unit();
        TilePosition { x: self.x + dx, y: self.y + dy }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ChunkPosition {
    pub x: i32,
    pub y: i32,
}

impl ChunkPosition {
    pub const fn first_tile(self) -> TilePosition {
        TilePosition { x: self.x * CHUNK_SIZE, y: self.y * CHUNK_SIZE }
    }
}

/// Factorio 2.0 uses 16 directions; the 4 cardinal ones are multiples of 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Direction(pub u8);

impl Direction {
    pub const NORTH: Direction = Direction(0);
    pub const EAST: Direction = Direction(4);
    pub const SOUTH: Direction = Direction(8);
    pub const WEST: Direction = Direction(12);
    pub const CARDINALS: [Direction; 4] = [Self::NORTH, Self::EAST, Self::SOUTH, Self::WEST];

    pub const fn rotate_cw(self) -> Direction {
        Direction((self.0 + 4) % 16)
    }
    pub const fn rotate_ccw(self) -> Direction {
        Direction((self.0 + 12) % 16)
    }
    pub const fn opposite(self) -> Direction {
        Direction((self.0 + 8) % 16)
    }
    pub const fn is_horizontal(self) -> bool {
        self.0 == 4 || self.0 == 12
    }

    /// Unit tile step for a cardinal direction.
    pub const fn unit(self) -> [i32; 2] {
        match self.0 {
            0 => [0, -1],
            4 => [1, 0],
            8 => [0, 1],
            12 => [-1, 0],
            _ => [0, 0],
        }
    }

    /// Rotates a vector given for a north-facing entity into this direction.
    pub const fn rotate_vec(self, v: [i32; 2]) -> [i32; 2] {
        let [x, y] = v;
        match self.0 {
            4 => [-y, x],
            8 => [-x, -y],
            12 => [y, -x],
            _ => [x, y],
        }
    }

    /// Index 0..4 for cardinal directions (N, E, S, W).
    pub const fn cardinal_index(self) -> usize {
        (self.0 / 4) as usize % 4
    }
}

/// Axis-aligned box in 1/256 tiles, relative to an entity position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BoundingBox {
    pub left_top: [i32; 2],
    pub right_bottom: [i32; 2],
}

impl BoundingBox {
    pub const fn new(left_top: [i32; 2], right_bottom: [i32; 2]) -> Self {
        BoundingBox { left_top, right_bottom }
    }

    pub fn rotated(self, d: Direction) -> BoundingBox {
        let a = d.rotate_vec(self.left_top);
        let b = d.rotate_vec(self.right_bottom);
        BoundingBox { left_top: [a[0].min(b[0]), a[1].min(b[1])], right_bottom: [a[0].max(b[0]), a[1].max(b[1])] }
    }

    /// Box in absolute map coordinates.
    pub fn at(self, p: MapPosition) -> Area {
        Area {
            left_top: MapPosition::new(p.x + self.left_top[0], p.y + self.left_top[1]),
            right_bottom: MapPosition::new(p.x + self.right_bottom[0], p.y + self.right_bottom[1]),
        }
    }

    pub fn is_empty(self) -> bool {
        self.left_top[0] >= self.right_bottom[0] || self.left_top[1] >= self.right_bottom[1]
    }
}

/// Absolute axis-aligned area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Area {
    pub left_top: MapPosition,
    pub right_bottom: MapPosition,
}

impl Area {
    /// Open-interval overlap test (touching edges do not overlap), as in Factorio.
    pub fn overlaps(&self, o: &Area) -> bool {
        self.left_top.x < o.right_bottom.x
            && o.left_top.x < self.right_bottom.x
            && self.left_top.y < o.right_bottom.y
            && o.left_top.y < self.right_bottom.y
    }

    pub fn contains(&self, p: MapPosition) -> bool {
        p.x >= self.left_top.x && p.x < self.right_bottom.x && p.y >= self.left_top.y && p.y < self.right_bottom.y
    }

    /// Every tile the area touches.
    pub fn tiles(&self) -> impl Iterator<Item = TilePosition> {
        let lt = self.left_top.tile();
        let rb = MapPosition::new(self.right_bottom.x - 1, self.right_bottom.y - 1).tile();
        (lt.y..=rb.y).flat_map(move |y| (lt.x..=rb.x).map(move |x| TilePosition { x, y }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_positions_floor_to_tiles() {
        assert_eq!(MapPosition { x: -1, y: 255 }.tile(), TilePosition { x: -1, y: 0 });
    }

    #[test]
    fn rotation_is_clockwise_with_y_down() {
        assert_eq!(Direction::EAST.rotate_vec([0, -256]), [256, 0]);
        assert_eq!(Direction::SOUTH.rotate_vec([0, -256]), [0, 256]);
        assert_eq!(Direction::WEST.rotate_vec([0, -256]), [-256, 0]);
    }

    #[test]
    fn area_tiles() {
        let b = BoundingBox::new([-346, -346], [346, 346]).at(MapPosition::tile_center(TilePosition::new(0, 0)));
        assert_eq!(b.tiles().count(), 9);
    }
}
