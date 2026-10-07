//! Transport belts, underground belts and splitters.
//!
//! Positions along a lane are in Factorio's belt units: 256 per straight tile. Items keep
//! at least [`ITEM_SPACING`] apart, belts move `speed` units per tick (8 for the basic
//! belt), so a lane carries 7.5 items/s and a belt 15 items/s, as in the game. Curve lanes
//! are 106 (inner) and 295 (outer) units long; sideloaded items enter the target lane at
//! 68 or 188 units depending on which feeder lane they come from.
//!
//! Lanes are updated front to back (a lane after the lane it feeds into), so a compressed
//! belt stays compressed, independent of entity ids.

use std::collections::{BTreeMap, BTreeSet};

use crate::map::{Direction, MapPosition, SUBTILES_PER_TILE, TilePosition};
use crate::proto::ItemId;
use crate::world::EntityId;

pub const STRAIGHT_LANE: i32 = 256;
pub const INNER_CURVE_LANE: i32 = 106;
pub const OUTER_CURVE_LANE: i32 = 295;
pub const ITEM_SPACING: i32 = 64;
pub const EARLY_SIDELOAD: i32 = STRAIGHT_LANE - 188;
pub const LATE_SIDELOAD: i32 = STRAIGHT_LANE - 68;

pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BeltItem {
    pub item: ItemId,
    pub pos: i32,
}

/// Items on one lane, sorted by position (ascending: back to front).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Lane {
    pub items: Vec<BeltItem>,
}

impl Lane {
    fn has_space_at(&self, pos: i32) -> bool {
        self.items.iter().all(|i| (i.pos - pos).abs() >= ITEM_SPACING)
    }

    fn insert_sorted(&mut self, item: BeltItem) {
        let at = self.items.partition_point(|i| i.pos < item.pos);
        self.items.insert(at, item);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BeltKind {
    Belt,
    UndergroundInput,
    UndergroundOutput,
    /// A splitter: two side-by-side halves (left, right), each with two lanes.
    Splitter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BeltShape {
    #[default]
    Straight,
    /// Turning left (counter-clockwise); the left lane is the inner one.
    CurveLeft,
    CurveRight,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BeltState {
    pub kind: BeltKind,
    pub direction: Direction,
    /// Centre of the entity.
    pub position: MapPosition,
    /// Positions per tick.
    pub speed: i32,
    pub shape: BeltShape,
    /// Two lanes, or four for a splitter (left half: 0, 1; right half: 2, 3).
    pub lanes: Vec<Lane>,
    /// Splitter: which output half each lane tries next.
    pub toggle: [bool; 2],
    /// Underground: maximum distance to its partner, in tiles.
    pub max_distance: u32,
    /// Underground input: distance to the paired output (0 = unpaired).
    pub underground_distance: u32,
}

impl BeltState {
    pub fn new(kind: BeltKind, position: MapPosition, direction: Direction, speed: i32) -> Self {
        let lanes = if kind == BeltKind::Splitter { 4 } else { 2 };
        BeltState {
            kind,
            direction,
            position,
            speed,
            shape: BeltShape::Straight,
            lanes: vec![Lane::default(); lanes],
            toggle: [false; 2],
            max_distance: 0,
            underground_distance: 0,
        }
    }

    pub fn lane_len(&self, lane: usize) -> i32 {
        match (self.shape, lane % 2) {
            (BeltShape::Straight, _) => STRAIGHT_LANE,
            (BeltShape::CurveLeft, LEFT) | (BeltShape::CurveRight, RIGHT) => INNER_CURVE_LANE,
            _ => OUTER_CURVE_LANE,
        }
    }

    /// Tiles covered by this belt entity; a splitter covers its left then right half.
    pub fn tiles(&self) -> Vec<TilePosition> {
        if self.kind == BeltKind::Splitter {
            let [lx, ly] = self.direction.rotate_ccw().unit();
            let half = SUBTILES_PER_TILE / 2;
            vec![self.position.offset(lx * half, ly * half).tile(), self.position.offset(-lx * half, -ly * half).tile()]
        } else {
            vec![self.position.tile()]
        }
    }

    pub fn item_count(&self) -> usize {
        self.lanes.iter().map(|l| l.items.len()).sum()
    }

    /// Removes and returns every item, e.g. when the belt is mined.
    pub fn take_all(&mut self) -> Vec<ItemId> {
        self.lanes.iter_mut().flat_map(|l| l.items.drain(..).map(|i| i.item)).collect()
    }

    /// Which lane of this belt is on the side of `from` (a point outside the belt, e.g. an
    /// inserter): returns (near, far). A point straight behind or ahead uses the right lane
    /// as the far lane, as Factorio does.
    pub fn lanes_seen_from(&self, tile: TilePosition, from: MapPosition) -> (usize, usize) {
        let centre = MapPosition::tile_center(tile);
        let [lx, ly] = self.direction.rotate_ccw().unit();
        let side = (from.x - centre.x) as i64 * lx as i64 + (from.y - centre.y) as i64 * ly as i64;
        let base = self.half_of(tile) * 2;
        if side > 0 { (base + LEFT, base + RIGHT) } else { (base + RIGHT, base + LEFT) }
    }

    /// For a point on the belt's tile, the lane on that side and the position along it.
    pub fn lane_at_point(&self, tile: TilePosition, p: MapPosition) -> (usize, i32) {
        let centre = MapPosition::tile_center(tile);
        let [dx, dy] = self.direction.unit();
        let [lx, ly] = self.direction.rotate_ccw().unit();
        let (ox, oy) = (p.x - centre.x, p.y - centre.y);
        let side = ox * lx + oy * ly;
        let along = ox * dx + oy * dy + SUBTILES_PER_TILE / 2;
        let lane = self.half_of(tile) * 2 + if side > 0 { LEFT } else { RIGHT };
        let len = self.lane_len(lane);
        (lane, (along * len / SUBTILES_PER_TILE).clamp(0, len - 1))
    }

    fn half_of(&self, tile: TilePosition) -> usize {
        if self.kind == BeltKind::Splitter && self.tiles()[1] == tile { 1 } else { 0 }
    }

    /// Inserts an item at `pos` on `lane` if there is room.
    pub fn try_insert(&mut self, lane: usize, pos: i32, item: ItemId) -> bool {
        let len = self.lane_len(lane);
        let pos = pos.clamp(0, len - 1);
        let l = &mut self.lanes[lane];
        if !l.has_space_at(pos) {
            return false;
        }
        l.insert_sorted(BeltItem { item, pos });
        true
    }

    /// Takes the front-most item matching `filter` from the first lane in `lanes` that has one.
    pub fn take(&mut self, lanes: &[usize], filter: impl Fn(ItemId) -> bool) -> Option<ItemId> {
        for &lane in lanes {
            let l = &mut self.lanes[lane];
            if let Some(i) = l.items.iter().rposition(|i| filter(i.item)) {
                return Some(l.items.remove(i).item);
            }
        }
        None
    }

    /// Offset of an item from the entity centre, in 1/256 tiles, for drawing.
    pub fn item_offset(&self, lane: usize, pos: i32) -> [i32; 2] {
        let len = self.lane_len(lane);
        let t = SUBTILES_PER_TILE;
        let lane_side = if lane % 2 == LEFT { t / 4 } else { -t / 4 };
        let half_offset = if self.kind == BeltKind::Splitter {
            let [lx, ly] = self.direction.rotate_ccw().unit();
            let s = if lane < 2 { t / 2 } else { -t / 2 };
            [lx * s, ly * s]
        } else {
            [0, 0]
        };
        let d = self.direction;
        let out_vec = d.unit();
        let in_dir = match self.shape {
            BeltShape::Straight => d,
            BeltShape::CurveLeft => d.rotate_cw(),
            BeltShape::CurveRight => d.rotate_ccw(),
        };
        let in_vec = in_dir.unit();
        // Walk the first half of the lane along the input direction, the second half along
        // the output direction.
        let frac = pos * t / len.max(1);
        let (a, b) = if frac < t / 2 { (frac - t / 2, 0) } else { (0, frac - t / 2) };
        let along = [in_vec[0] * a + out_vec[0] * b, in_vec[1] * a + out_vec[1] * b];
        let side_in = in_dir.rotate_ccw().unit();
        let side_out = d.rotate_ccw().unit();
        let side = if frac < t / 2 { side_in } else { side_out };
        [along[0] + side[0] * lane_side + half_offset[0], along[1] + side[1] * lane_side + half_offset[1]]
    }
}

/// Where items leaving a lane go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Next {
    None,
    /// Continue on another lane, entering at `entry` (0 for a straight connection).
    Lane {
        belt: EntityId,
        lane: usize,
        entry: i32,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BeltSystem {
    pub belts: BTreeMap<EntityId, BeltState>,
    by_tile: BTreeMap<TilePosition, EntityId>,
    /// Successors per (belt, lane). Splitter lanes have two (one per output half).
    next: BTreeMap<(EntityId, usize), Vec<Next>>,
    order: Vec<(EntityId, usize)>,
    dirty: bool,
}

impl BeltSystem {
    pub fn add(&mut self, id: EntityId, state: BeltState) {
        for t in state.tiles() {
            self.by_tile.insert(t, id);
        }
        self.belts.insert(id, state);
        self.dirty = true;
    }

    pub fn remove(&mut self, id: EntityId) -> Option<BeltState> {
        let state = self.belts.remove(&id)?;
        for t in state.tiles() {
            self.by_tile.remove(&t);
        }
        self.dirty = true;
        Some(state)
    }

    pub fn set_direction(&mut self, id: EntityId, d: Direction) {
        if let Some(b) = self.belts.get_mut(&id) {
            b.direction = d;
            self.dirty = true;
        }
    }

    pub fn at_tile(&self, t: TilePosition) -> Option<EntityId> {
        self.by_tile.get(&t).copied()
    }

    pub fn get(&self, id: EntityId) -> Option<&BeltState> {
        self.belts.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut BeltState> {
        self.belts.get_mut(&id)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Does the belt-like entity at `from` (facing `d`) output into tile `to`?
    fn outputs_into(&self, from_tile: TilePosition, to: TilePosition) -> bool {
        let Some(id) = self.at_tile(from_tile) else { return false };
        let b = &self.belts[&id];
        b.kind != BeltKind::UndergroundInput && from_tile.step(b.direction) == to
    }

    fn rebuild(&mut self) {
        self.dirty = false;
        let ids: Vec<EntityId> = self.belts.keys().copied().collect();

        // Pair underground belts: an input pairs with the nearest output ahead of it,
        // facing the same way, within range, with no other input in between.
        let mut pair: BTreeMap<EntityId, (EntityId, u32)> = BTreeMap::new();
        for &id in &ids {
            let b = &self.belts[&id];
            if b.kind != BeltKind::UndergroundInput {
                continue;
            }
            let max = b.max_distance.max(1);
            let mut t = b.position.tile();
            for dist in 1..=max {
                t = t.step(b.direction);
                if let Some(other) = self.at_tile(t) {
                    let o = &self.belts[&other];
                    if o.direction == b.direction && o.kind == BeltKind::UndergroundOutput {
                        pair.insert(id, (other, dist));
                        break;
                    }
                    if o.direction == b.direction && o.kind == BeltKind::UndergroundInput {
                        break;
                    }
                }
            }
        }

        // Curves: a plain belt with exactly one side input and no input from behind.
        for &id in &ids {
            let b = &self.belts[&id];
            if b.kind != BeltKind::Belt {
                continue;
            }
            let t = b.position.tile();
            let d = b.direction;
            let behind = self.outputs_into(t.step(d.opposite()), t);
            let left = self.outputs_into(t.step(d.rotate_ccw()), t);
            let right = self.outputs_into(t.step(d.rotate_cw()), t);
            let shape = match (behind, left, right) {
                (false, true, false) => BeltShape::CurveLeft,
                (false, false, true) => BeltShape::CurveRight,
                _ => BeltShape::Straight,
            };
            let b = self.belts.get_mut(&id).unwrap();
            if b.shape != shape {
                // Keep items within the new lane lengths.
                b.shape = shape;
                for lane in 0..2 {
                    let len = b.lane_len(lane);
                    for item in b.lanes[lane].items.iter_mut() {
                        item.pos = item.pos.min(len);
                    }
                }
            }
        }

        self.next.clear();
        for &id in &ids {
            let b = &self.belts[&id];
            let tiles = b.tiles();
            for (half, tile) in tiles.iter().enumerate() {
                for lane in 0..2 {
                    let idx = half * 2 + lane;
                    let next = if b.kind == BeltKind::UndergroundInput {
                        match pair.get(&id) {
                            Some((exit, _)) => Next::Lane { belt: *exit, lane, entry: 0 },
                            None => Next::None,
                        }
                    } else {
                        self.next_from(*tile, b.direction, lane)
                    };
                    self.next.entry((id, idx)).or_default().push(next);
                }
            }
            if b.kind == BeltKind::Splitter {
                // Each input lane can go to either output half.
                for lane in 0..2 {
                    let a = self.next[&(id, lane)][0];
                    let c = self.next[&(id, 2 + lane)][0];
                    self.next.insert((id, lane), vec![a, c]);
                    self.next.insert((id, 2 + lane), vec![a, c]);
                }
            }
        }
        for b in self.belts.values_mut() {
            b.underground_distance = 0;
        }
        for (&id, &(_, dist)) in &pair {
            self.belts.get_mut(&id).unwrap().underground_distance = dist;
        }

        // Order lanes so each is updated after everything it feeds into.
        let mut preds: BTreeMap<(EntityId, usize), Vec<(EntityId, usize)>> = BTreeMap::new();
        let mut out_degree: BTreeMap<(EntityId, usize), usize> = BTreeMap::new();
        for (key, nexts) in &self.next {
            let mut targets = BTreeSet::new();
            for n in nexts {
                if let Next::Lane { belt, lane, .. } = n {
                    targets.insert((*belt, *lane));
                }
            }
            out_degree.insert(*key, targets.len());
            for t in targets {
                preds.entry(t).or_default().push(*key);
            }
        }
        let mut ready: BTreeSet<(EntityId, usize)> =
            out_degree.iter().filter(|(_, d)| **d == 0).map(|(k, _)| *k).collect();
        let mut order = Vec::with_capacity(out_degree.len());
        let mut done: BTreeSet<(EntityId, usize)> = BTreeSet::new();
        loop {
            while let Some(k) = ready.pop_first() {
                if !done.insert(k) {
                    continue;
                }
                order.push(k);
                for p in preds.get(&k).into_iter().flatten() {
                    let d = out_degree.get_mut(p).unwrap();
                    *d = d.saturating_sub(1);
                    if *d == 0 && !done.contains(p) {
                        ready.insert(*p);
                    }
                }
            }
            // Loops: start anywhere (lowest key) and continue.
            match out_degree.keys().find(|k| !done.contains(k)) {
                Some(k) => {
                    ready.insert(*k);
                }
                None => break,
            }
        }
        self.order = order;
    }

    fn next_from(&self, tile: TilePosition, d: Direction, lane: usize) -> Next {
        let to = tile.step(d);
        let Some(target) = self.at_tile(to) else { return Next::None };
        let t = &self.belts[&target];
        let td = t.direction;
        match t.kind {
            BeltKind::Splitter => {
                if td == d {
                    let half = if t.tiles()[0] == to { 0 } else { 1 };
                    Next::Lane { belt: target, lane: half * 2 + lane, entry: 0 }
                } else {
                    Next::None
                }
            }
            BeltKind::UndergroundOutput if td == d => Next::None,
            _ if td == d => Next::Lane { belt: target, lane, entry: 0 },
            _ if td == d.opposite() => Next::None,
            BeltKind::Belt if t.shape != BeltShape::Straight => Next::Lane { belt: target, lane, entry: 0 },
            BeltKind::UndergroundInput | BeltKind::UndergroundOutput | BeltKind::Belt => {
                // Sideload onto the target lane nearest to us. Our lane nearer the target's
                // upstream end enters earlier.
                let target_lane = if td.rotate_ccw() == d.opposite() { LEFT } else { RIGHT };
                let upstream = td.opposite();
                let our_lane_side = if lane == LEFT { d.rotate_ccw() } else { d.rotate_cw() };
                let entry = if our_lane_side == upstream { EARLY_SIDELOAD } else { LATE_SIDELOAD };
                Next::Lane { belt: target, lane: target_lane, entry }
            }
        }
    }

    /// Moves every item by one tick.
    pub fn update(&mut self) {
        if self.dirty {
            self.rebuild();
        }
        let order = std::mem::take(&mut self.order);
        for &(id, lane) in &order {
            self.update_lane(id, lane);
        }
        self.order = order;
    }

    fn update_lane(&mut self, id: EntityId, lane: usize) {
        let Some(b) = self.belts.get(&id) else { return };
        if b.lanes[lane].items.is_empty() {
            return;
        }
        let speed = b.speed;
        let len = b.lane_len(lane);
        let nexts = self.next.get(&(id, lane)).cloned().unwrap_or_default();
        let is_splitter = b.kind == BeltKind::Splitter;

        // How far the front item may go. Positions are half-open: an item at `len` has
        // already left the lane, so a dead end holds items up to `len - 1` (4 per lane).
        let front_limit = match (nexts.first(), is_splitter) {
            (Some(Next::Lane { belt, lane: nl, entry: 0 }), false) => {
                let first = self.belts[belt].lanes[*nl].items.first().map(|i| i.pos);
                match first {
                    Some(p) => len + p - ITEM_SPACING,
                    None => len + self.belts[belt].lane_len(*nl) - 1,
                }
            }
            (Some(_), _) => {
                // Sideload or splitter: may leave only if some target has room now.
                let room = nexts.iter().any(|n| match n {
                    Next::Lane { belt, lane: nl, entry } => self.belts[belt].lanes[*nl].has_space_at(*entry),
                    Next::None => false,
                });
                if room { len } else { len - 1 }
            }
            (None, _) => len - 1,
        };

        let b = self.belts.get_mut(&id).unwrap();
        let items = &mut b.lanes[lane].items;
        let mut limit = front_limit;
        for item in items.iter_mut().rev() {
            let target = (item.pos + speed).min(limit);
            if target > item.pos {
                item.pos = target;
            }
            limit = item.pos - ITEM_SPACING;
        }

        // Transfer items that reached the end.
        loop {
            let b = self.belts.get(&id).unwrap();
            let Some(front) = b.lanes[lane].items.last().copied() else { break };
            if front.pos < len {
                break;
            }
            let moved = if is_splitter {
                let half_toggle = lane % 2;
                let first = b.toggle[half_toggle] as usize;
                let mut done = false;
                for k in [first, 1 - first] {
                    if let Some(Next::Lane { belt, lane: nl, entry }) = nexts.get(k).copied()
                        && self.belts.get_mut(&belt).unwrap().try_insert(nl, entry, front.item)
                    {
                        let b = self.belts.get_mut(&id).unwrap();
                        b.toggle[half_toggle] = k == 0;
                        done = true;
                        break;
                    }
                }
                done
            } else {
                match nexts.first().copied() {
                    Some(Next::Lane { belt, lane: nl, entry: 0 }) => {
                        let overflow = front.pos - len;
                        let t = self.belts.get_mut(&belt).unwrap();
                        let ok = t.lanes[nl].items.first().is_none_or(|i| i.pos - overflow >= ITEM_SPACING);
                        if ok {
                            t.lanes[nl].insert_sorted(BeltItem { item: front.item, pos: overflow });
                        }
                        ok
                    }
                    Some(Next::Lane { belt, lane: nl, entry }) => {
                        self.belts.get_mut(&belt).unwrap().try_insert(nl, entry, front.item)
                    }
                    _ => false,
                }
            };
            if !moved {
                let b = self.belts.get_mut(&id).unwrap();
                b.lanes[lane].items.last_mut().unwrap().pos = len - 1;
                break;
            }
            self.belts.get_mut(&id).unwrap().lanes[lane].items.pop();
        }
    }
}
