//! Fluid networks (offshore pump → pipes → boiler → steam engine) and electric networks.
//!
//! Fluids follow Factorio 2.0's segment model: every connected set of fluid boxes acts
//! as one tank whose contents are shared in proportion to box volume. Electric networks
//! are poles linked within wire reach; machines inside a pole's supply area draw from the
//! network, and generators supply what is demanded, limited by the steam they have.

use std::collections::BTreeMap;

use crate::energy::EnergyState;
use crate::fixed::Fixed;
use crate::map::{Direction, MapPosition, SUBTILES_PER_TILE, TilePosition};
use crate::proto::{Energy, EnergySource, EntityData, EntityProto, EntityProtoId, FluidBoxProto, FluidId, PrototypeDb};
use crate::world::{EntityId, EntityState, Simulation};

type Connection = (usize, TilePosition, Direction, Option<u32>);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FluidBox {
    pub fluid: Option<FluidId>,
    pub amount: Fixed,
    pub temperature: Fixed,
}

/// Runtime state of pipes, pumps, boilers and generators.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FluidEntity {
    pub energy: EnergyState,
    pub boxes: Vec<FluidBox>,
    /// Energy produced (generators) or consumed (boilers) last tick, for display.
    pub last_power: Energy,
}

impl FluidEntity {
    pub fn new(proto: &EntityProto) -> Self {
        let (energy, n) = match &proto.data {
            EntityData::Boiler { energy_source, .. } => (EnergyState::for_source(energy_source), 2),
            _ => (EnergyState::None, 1),
        };
        FluidEntity { energy, boxes: vec![FluidBox::default(); n], last_power: Fixed::ZERO }
    }
}

pub fn fluid_boxes(proto: &EntityProto) -> Vec<&FluidBoxProto> {
    match &proto.data {
        EntityData::Pipe { fluid_box }
        | EntityData::OffshorePump { fluid_box, .. }
        | EntityData::Generator { fluid_box, .. } => {
            vec![fluid_box]
        }
        EntityData::Boiler { fluid_box, output_fluid_box, .. } => vec![fluid_box, output_fluid_box],
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FluidNetwork {
    pub members: Vec<(EntityId, usize)>,
    pub capacity: Fixed,
    pub fluid: Option<FluidId>,
    pub amount: Fixed,
    pub temperature: Fixed,
}

impl FluidNetwork {
    fn add(&mut self, fluid: FluidId, amount: Fixed, temperature: Fixed) -> Fixed {
        if self.fluid.is_some_and(|f| f != fluid) {
            return Fixed::ZERO;
        }
        let n = amount.min(self.capacity - self.amount).max(Fixed::ZERO);
        if !n.is_positive() {
            return Fixed::ZERO;
        }
        let total = self.amount + n;
        self.temperature = (self.temperature * self.amount + temperature * n) / total;
        self.amount = total;
        self.fluid = Some(fluid);
        n
    }

    fn take(&mut self, amount: Fixed) -> Fixed {
        let n = amount.min(self.amount).max(Fixed::ZERO);
        self.amount -= n;
        n
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElectricNetwork {
    pub poles: Vec<EntityId>,
    pub consumers: Vec<EntityId>,
    pub generators: Vec<EntityId>,
    /// Last tick's demand and production, in joules per tick.
    pub demand: Energy,
    pub production: Energy,
    pub capacity: Energy,
}

impl ElectricNetwork {
    pub fn satisfaction(&self) -> Fixed {
        if self.demand.is_positive() { (self.production / self.demand).min(Fixed::ONE) } else { Fixed::ONE }
    }
}

/// Average power per entity type over one sample period, in joules per tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PowerSample {
    pub consumption: BTreeMap<EntityProtoId, Energy>,
    pub production: BTreeMap<EntityProtoId, Energy>,
}

impl PowerSample {
    pub fn total_consumption(&self) -> Energy {
        self.consumption.values().fold(Fixed::ZERO, |a, b| a + *b)
    }
    pub fn total_production(&self) -> Energy {
        self.production.values().fold(Fixed::ZERO, |a, b| a + *b)
    }
    fn add(&mut self, o: &PowerSample) {
        for (k, v) in &o.consumption {
            *self.consumption.entry(*k).or_insert(Fixed::ZERO) += *v;
        }
        for (k, v) in &o.production {
            *self.production.entry(*k).or_insert(Fixed::ZERO) += *v;
        }
    }
    fn divided(&self, n: i64) -> PowerSample {
        PowerSample {
            consumption: self.consumption.iter().map(|(k, v)| (*k, v.div_int(n))).collect(),
            production: self.production.iter().map(|(k, v)| (*k, v.div_int(n))).collect(),
        }
    }
}

/// Samples kept per time range, like the game's power graphs.
pub const STAT_SAMPLES: usize = 300;
/// Ticks per sample for the 5 s, 1 min and 10 min ranges.
pub const STAT_PERIODS: [u32; 3] = [1, 12, 120];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatSeries {
    pub samples: std::collections::VecDeque<PowerSample>,
    acc: PowerSample,
    acc_ticks: u32,
}

/// Power history of one electric network, for the network window's graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkStats {
    pub series: [StatSeries; 3],
}

impl NetworkStats {
    fn record(&mut self, tick: &PowerSample) {
        for (series, period) in self.series.iter_mut().zip(STAT_PERIODS) {
            series.acc.add(tick);
            series.acc_ticks += 1;
            if series.acc_ticks == period {
                series.samples.push_back(series.acc.divided(period as i64));
                if series.samples.len() > STAT_SAMPLES {
                    series.samples.pop_front();
                }
                series.acc = PowerSample::default();
                series.acc_ticks = 0;
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PowerSystem {
    dirty: bool,
    pub fluid_networks: Vec<FluidNetwork>,
    pub fluid_network_of: BTreeMap<(EntityId, usize), usize>,
    pub electric_networks: Vec<ElectricNetwork>,
    pub electric_network_of: BTreeMap<EntityId, usize>,
    /// Energy delivered to each electric consumer last tick (joules), for display.
    pub last_consumption: BTreeMap<EntityId, Energy>,
    /// History per network, keyed by the network's first (lowest id) pole so it survives
    /// rebuilds when buildings are added or removed.
    pub stats: BTreeMap<EntityId, NetworkStats>,
}

impl PowerSystem {
    /// Statistics for the network the entity (pole or machine) belongs to.
    pub fn stats_for(&self, id: EntityId) -> Option<&NetworkStats> {
        let n = self.electric_network_of.get(&id)?;
        self.stats.get(self.electric_networks[*n].poles.first()?)
    }
}

impl PowerSystem {
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

/// Tile collision test for building, using `tile_buildability_rules` when present.
pub(crate) fn buildable_on_tiles(
    sim: &Simulation,
    proto: &EntityProto,
    position: MapPosition,
    direction: Direction,
) -> bool {
    let db = &sim.db;
    let layers = |t: TilePosition| sim.surface.tile(t).map(|id| db.tile(id).collision_mask.layers);
    if !proto.tile_rules.is_empty() {
        return proto.tile_rules.iter().all(|rule| {
            rule.area.rotated(direction).at(position).tiles().all(|t| match layers(t) {
                Some(l) => (rule.required == 0 || l & rule.required != 0) && l & rule.colliding == 0,
                None => false,
            })
        });
    }
    Simulation::footprint(proto, position, direction).tiles().all(|t| match sim.surface.tile(t) {
        Some(id) => !db.tile(id).collision_mask.collides(&proto.collision_mask),
        None => false,
    })
}

/// Connection points of an entity's fluid boxes: (box, tile inside the entity, facing,
/// underground reach).
fn connections(db: &PrototypeDb, e: &crate::world::Entity) -> Vec<Connection> {
    let proto = db.entity(e.proto);
    let mut out = Vec::new();
    for (bi, fb) in fluid_boxes(proto).into_iter().enumerate() {
        for c in &fb.connections {
            let [x, y] = e.direction.rotate_vec(c.position);
            let tile = e.position.offset(x, y).tile();
            let facing = Direction((c.direction.0 + e.direction.0) % 16);
            out.push((bi, tile, facing, c.underground_max_distance));
        }
    }
    out
}

fn find(parent: &mut BTreeMap<(EntityId, usize), (EntityId, usize)>, k: (EntityId, usize)) -> (EntityId, usize) {
    let mut r = k;
    while parent[&r] != r {
        r = parent[&r];
    }
    let mut c = k;
    while parent[&c] != r {
        let n = parent[&c];
        parent.insert(c, r);
        c = n;
    }
    r
}

fn rebuild(sim: &mut Simulation) {
    let db = sim.db.clone();
    let p = &mut sim.power;
    p.dirty = false;

    // --- fluids ---
    let mut parent: BTreeMap<(EntityId, usize), (EntityId, usize)> = BTreeMap::new();
    let mut conns: BTreeMap<EntityId, Vec<Connection>> = BTreeMap::new();
    for (id, e) in &sim.entities {
        if let EntityState::Fluid(f) = &e.state {
            for bi in 0..f.boxes.len() {
                parent.insert((*id, bi), (*id, bi));
            }
            conns.insert(*id, connections(&db, e));
        }
    }
    let by_tile: BTreeMap<TilePosition, EntityId> =
        conns.keys().flat_map(|id| conns[id].iter().map(move |(_, t, _, _)| (*t, *id))).collect();
    for (id, cs) in &conns {
        for (bi, tile, facing, under) in cs {
            let reach = under.unwrap_or(1);
            let mut t = *tile;
            for _ in 0..reach {
                t = t.step(*facing);
                let Some(other) = by_tile.get(&t) else { continue };
                let found = conns[other]
                    .iter()
                    .find(|(_, ot, of, ou)| *ot == t && *of == facing.opposite() && ou.is_some() == under.is_some());
                if let Some((obi, _, _, _)) = found {
                    let a = find(&mut parent, (*id, *bi));
                    let b = find(&mut parent, (*other, *obi));
                    if a != b {
                        parent.insert(a.max(b), a.min(b));
                    }
                    break;
                }
                if under.is_some()
                    && conns[other].iter().any(|(_, ot, of, ou)| *ot == t && ou.is_some() && *of == *facing)
                {
                    break;
                }
            }
        }
    }
    let keys: Vec<(EntityId, usize)> = parent.keys().copied().collect();
    let mut net_index: BTreeMap<(EntityId, usize), usize> = BTreeMap::new();
    let mut networks: Vec<FluidNetwork> = Vec::new();
    for k in keys {
        let root = find(&mut parent, k);
        let idx = *net_index.entry(root).or_insert_with(|| {
            networks.push(FluidNetwork::default());
            networks.len() - 1
        });
        let e = &sim.entities[&k.0];
        let volume = fluid_boxes(db.entity(e.proto))[k.1].volume;
        networks[idx].members.push(k);
        networks[idx].capacity += volume;
    }
    let mut network_of = BTreeMap::new();
    for (i, n) in networks.iter().enumerate() {
        for m in &n.members {
            network_of.insert(*m, i);
        }
    }
    p.fluid_networks = networks;
    p.fluid_network_of = network_of;

    // --- electricity ---
    let poles: Vec<(EntityId, MapPosition, Fixed, Fixed)> = sim
        .entities
        .iter()
        .filter_map(|(id, e)| match &db.entity(e.proto).data {
            EntityData::ElectricPole { supply_area_distance, maximum_wire_distance } => {
                Some((*id, e.position, *supply_area_distance, *maximum_wire_distance))
            }
            _ => None,
        })
        .collect();
    let mut pole_parent: Vec<usize> = (0..poles.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..poles.len() {
        for j in i + 1..poles.len() {
            let reach = poles[i].3.min(poles[j].3).mul_int(SUBTILES_PER_TILE as i64).floor_int();
            if poles[i].1.distance_sq(poles[j].1) <= reach * reach {
                let (a, b) = (root(&mut pole_parent, i), root(&mut pole_parent, j));
                if a != b {
                    pole_parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut enets: Vec<ElectricNetwork> = Vec::new();
    let mut root_net: BTreeMap<usize, usize> = BTreeMap::new();
    let mut enet_of: BTreeMap<EntityId, usize> = BTreeMap::new();
    for (i, pole) in poles.iter().enumerate() {
        let r = root(&mut pole_parent, i);
        let n = *root_net.entry(r).or_insert_with(|| {
            enets.push(ElectricNetwork::default());
            enets.len() - 1
        });
        enets[n].poles.push(pole.0);
        enet_of.insert(pole.0, n);
    }
    for (id, e) in &sim.entities {
        let proto = db.entity(e.proto);
        let is_gen = matches!(proto.data, EntityData::Generator { .. });
        let is_consumer = matches!(proto.energy_source(), Some(EnergySource::Electric { .. }));
        if !is_gen && !is_consumer {
            continue;
        }
        let fp = Simulation::footprint(proto, e.position, e.direction);
        let net = poles.iter().find_map(|(pid, pp, supply, _)| {
            let s = supply.mul_int(SUBTILES_PER_TILE as i64).floor_int() as i32;
            let area = crate::map::Area { left_top: pp.offset(-s, -s), right_bottom: pp.offset(s, s) };
            area.overlaps(&fp).then(|| enet_of[pid])
        });
        if let Some(n) = net {
            if is_gen {
                enets[n].generators.push(*id);
            } else {
                enets[n].consumers.push(*id);
            }
            enet_of.insert(*id, n);
        }
    }
    p.electric_networks = enets;
    p.electric_network_of = enet_of;
}

/// Energy an electric consumer wants buffered per tick.
pub fn electric_buffer_capacity(proto: &EntityProto) -> Energy {
    let drain = match proto.energy_source() {
        Some(EnergySource::Electric { drain, .. }) => *drain,
        _ => return Fixed::ZERO,
    };
    match &proto.data {
        EntityData::MiningDrill { energy_usage, .. } | EntityData::CraftingMachine { energy_usage, .. } => {
            *energy_usage + drain
        }
        EntityData::Inserter { rotation_speed, extension_speed, energy_per_rotation, energy_per_movement, .. } => {
            *rotation_speed * *energy_per_rotation + *extension_speed * *energy_per_movement + drain
        }
        _ => drain,
    }
}

fn electric_buffer(state: &mut EntityState) -> Option<&mut Energy> {
    let energy = match state {
        EntityState::Drill(d) => &mut d.energy,
        EntityState::Crafter(c) => &mut c.energy,
        EntityState::Inserter(i) => &mut i.energy,
        _ => return None,
    };
    match energy {
        EnergyState::Electric { buffer } => Some(buffer),
        _ => None,
    }
}

pub(crate) fn update(sim: &mut Simulation) {
    if sim.power.dirty {
        rebuild(sim);
    }
    let db = sim.db.clone();

    // Load fluid network contents from the boxes.
    let mut nets = std::mem::take(&mut sim.power.fluid_networks);
    for n in nets.iter_mut() {
        n.fluid = None;
        n.amount = Fixed::ZERO;
        n.temperature = Fixed::ZERO;
        let mut heat = Fixed::ZERO;
        for (id, bi) in &n.members {
            if let EntityState::Fluid(f) = &sim.entities[id].state {
                let b = f.boxes[*bi];
                if let Some(fl) = b.fluid
                    && b.amount.is_positive()
                {
                    n.fluid.get_or_insert(fl);
                    n.amount += b.amount;
                    heat += b.amount * b.temperature;
                }
            }
        }
        if n.amount.is_positive() {
            n.temperature = heat / n.amount;
        }
    }
    let net_of = |k: (EntityId, usize)| sim.power.fluid_network_of.get(&k).copied();

    let ids: Vec<EntityId> = sim.entities.keys().copied().collect();
    for &id in &ids {
        let e = &sim.entities[&id];
        let proto = db.entity(e.proto);
        match &proto.data {
            EntityData::OffshorePump { pumping_speed, fluid_source_offset, .. } => {
                let [x, y] = e.direction.rotate_vec(*fluid_source_offset);
                let fluid = sim.surface.tile(e.position.offset(x, y).tile()).and_then(|t| db.tile(t).fluid);
                if let (Some(fl), Some(n)) = (fluid, net_of((id, 0))) {
                    let temp = db.fluid(fl).default_temperature;
                    nets[n].add(fl, *pumping_speed, temp);
                }
            }
            EntityData::Boiler { energy_consumption, energy_source, target_temperature, output_fluid_box, .. } => {
                let (Some(nin), Some(nout)) = (net_of((id, 0)), net_of((id, 1))) else { continue };
                let (Some(_), Some(steam)) = (nets[nin].fluid, output_fluid_box.filter) else { continue };
                let in_temp = nets[nin].temperature;
                let dt = *target_temperature - in_temp;
                // The boiler's energy goes into the output fluid, so its heat capacity applies
                // (steam: 0.2 kJ/°C, giving 60 units/s at 1.8 MW from 15 to 165 °C).
                let hc = db.fluid(steam).heat_capacity;
                if !dt.is_positive() || !hc.is_positive() {
                    continue;
                }
                let per_unit = dt * hc;
                let free = nets[nout].capacity - nets[nout].amount;
                let units = (*energy_consumption / per_unit).min(nets[nin].amount).min(free);
                if !units.is_positive() {
                    continue;
                }
                let need = units * per_unit;
                let EntityState::Fluid(f) = &mut sim.entities.get_mut(&id).unwrap().state else { continue };
                let got = f.energy.draw(&db, energy_source, need);
                f.last_power = got;
                let units = if got >= need { units } else { got / per_unit };
                let units = nets[nin].take(units);
                nets[nout].add(steam, units, *target_temperature);
            }
            _ => {}
        }
    }

    // Electricity.
    sim.power.last_consumption.clear();
    let mut enets = std::mem::take(&mut sim.power.electric_networks);
    for en in enets.iter_mut() {
        let mut demand = Fixed::ZERO;
        let mut wants: Vec<(EntityId, Energy)> = Vec::new();
        for id in &en.consumers {
            let e = sim.entities.get_mut(id).unwrap();
            let cap = electric_buffer_capacity(db.entity(e.proto));
            if let Some(buf) = electric_buffer(&mut e.state) {
                let w = (cap - *buf).max(Fixed::ZERO);
                demand += w;
                wants.push((*id, w));
            }
        }
        // Each generator can turn at most `fluid_usage_per_tick` of steam into power.
        let mut offers: Vec<(EntityId, usize, Fixed, Energy)> = Vec::new();
        let mut supply = Fixed::ZERO;
        let mut reserved: BTreeMap<usize, Fixed> = BTreeMap::new();
        for id in &en.generators {
            let e = &sim.entities[id];
            let EntityData::Generator { fluid_usage_per_tick, maximum_temperature, effectivity, fluid_box } =
                &db.entity(e.proto).data
            else {
                continue;
            };
            let Some(n) = net_of((*id, 0)) else { continue };
            let Some(fl) = nets[n].fluid else { continue };
            if fluid_box.filter.is_some_and(|f| f != fl) {
                continue;
            }
            if let Some(min) = fluid_box.minimum_temperature
                && nets[n].temperature < min
            {
                continue;
            }
            let fp = db.fluid(fl);
            let temp = nets[n].temperature.min(*maximum_temperature);
            let per_unit = (temp - fp.default_temperature) * fp.heat_capacity * *effectivity;
            if !per_unit.is_positive() {
                continue;
            }
            let used = reserved.entry(n).or_insert(Fixed::ZERO);
            let units = (*fluid_usage_per_tick).min(nets[n].amount - *used).max(Fixed::ZERO);
            *used += units;
            let energy = units * per_unit;
            supply += energy;
            offers.push((*id, n, units, energy));
        }
        let mut sample = PowerSample::default();
        let given = demand.min(supply);
        en.demand = demand;
        en.capacity = supply;
        en.production = given;
        if demand.is_positive() {
            let sat = given / demand;
            for (id, w) in wants {
                let e = sim.entities.get_mut(&id).unwrap();
                if let Some(buf) = electric_buffer(&mut e.state) {
                    *buf += w * sat;
                }
                sim.power.last_consumption.insert(id, w * sat);
                *sample.consumption.entry(e.proto).or_insert(Fixed::ZERO) += w * sat;
            }
        }
        let load = if supply.is_positive() { given / supply } else { Fixed::ZERO };
        for (id, n, units, energy) in offers {
            nets[n].take(units * load);
            let e = sim.entities.get_mut(&id).unwrap();
            *sample.production.entry(e.proto).or_insert(Fixed::ZERO) += energy * load;
            if let EntityState::Fluid(f) = &mut e.state {
                f.last_power = energy * load;
            }
        }
        if let Some(first) = en.poles.first() {
            sim.power.stats.entry(*first).or_default().record(&sample);
        }
    }
    sim.power.electric_networks = enets;

    // Write fluid contents back, shared by volume.
    for n in &nets {
        let mut left = n.amount;
        for (i, (id, bi)) in n.members.iter().enumerate() {
            let volume = fluid_boxes(db.entity(sim.entities[id].proto))[*bi].volume;
            let share = if i + 1 == n.members.len() || !n.capacity.is_positive() {
                left
            } else {
                n.amount * volume / n.capacity
            };
            left -= share;
            if let EntityState::Fluid(f) = &mut sim.entities.get_mut(id).unwrap().state {
                f.boxes[*bi] = FluidBox {
                    fluid: if share.is_positive() { n.fluid } else { None },
                    amount: share,
                    temperature: n.temperature,
                };
            }
        }
    }
    sim.power.fluid_networks = nets;
}
