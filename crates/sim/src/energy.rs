//! Burner fuel and electric energy buffers.

use crate::Fixed;
use crate::inventory::Inventory;
use crate::proto::{Energy, EnergySource, ItemId, PrototypeDb};

/// State of a `burner` energy source: a fuel inventory plus the item currently burning.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Burner {
    pub fuel: Inventory,
    pub burnt: Inventory,
    pub currently_burning: Option<ItemId>,
    /// Energy left in the item currently burning.
    pub remaining: Energy,
}

impl Burner {
    pub fn new(fuel_inventory_size: u32) -> Self {
        Burner { fuel: Inventory::new(fuel_inventory_size), ..Default::default() }
    }

    /// Takes `amount` of energy, starting a new fuel item when the current one runs out.
    /// Returns the energy actually obtained (less than `amount` when out of fuel).
    pub fn consume(&mut self, db: &PrototypeDb, amount: Energy, effectivity: Fixed) -> Energy {
        // Burning `x` joules of fuel yields `x * effectivity` joules of work.
        let mut fuel_needed = if effectivity == Fixed::ONE { amount } else { amount / effectivity };
        let mut got = Fixed::ZERO;
        loop {
            let take = fuel_needed.min(self.remaining);
            self.remaining -= take;
            fuel_needed -= take;
            got += take;
            if !fuel_needed.is_positive() {
                break;
            }
            if !self.start_next(db) {
                break;
            }
        }
        if effectivity == Fixed::ONE { got } else { got * effectivity }
    }

    /// True when there is energy available right now or fuel to start burning.
    pub fn has_fuel(&self) -> bool {
        self.remaining.is_positive() || !self.fuel.is_empty()
    }

    fn start_next(&mut self, db: &PrototypeDb) -> bool {
        let Some(item) = self.fuel.first_item() else {
            self.currently_burning = None;
            return false;
        };
        let Some(fuel) = db.item(item).fuel.clone() else { return false };
        if let Some(burnt) = fuel.burnt_result
            && self.burnt.space_for(db, burnt) == 0
        {
            return false;
        }
        self.fuel.remove(item, 1);
        if let Some(burnt) = fuel.burnt_result {
            self.burnt.insert(db, burnt, 1);
        }
        self.currently_burning = Some(item);
        self.remaining += fuel.value;
        true
    }

    /// Whether `item` is a fuel this burner accepts.
    pub fn accepts(db: &PrototypeDb, source: &EnergySource, item: ItemId) -> bool {
        match (source, &db.item(item).fuel) {
            (EnergySource::Burner { fuel_categories, .. }, Some(f)) => fuel_categories.contains(&f.category),
            _ => false,
        }
    }
}

/// The energy side of an entity: burner fuel, an electric buffer, or nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum EnergyState {
    #[default]
    None,
    Burner(Burner),
    Electric {
        buffer: Energy,
    },
}

impl EnergyState {
    pub fn for_source(source: &EnergySource) -> Self {
        match source {
            EnergySource::Burner { fuel_inventory_size, .. } => EnergyState::Burner(Burner::new(*fuel_inventory_size)),
            EnergySource::Electric { .. } => EnergyState::Electric { buffer: Fixed::ZERO },
            _ => EnergyState::None,
        }
    }

    pub fn burner(&self) -> Option<&Burner> {
        match self {
            EnergyState::Burner(b) => Some(b),
            _ => None,
        }
    }

    pub fn burner_mut(&mut self) -> Option<&mut Burner> {
        match self {
            EnergyState::Burner(b) => Some(b),
            _ => None,
        }
    }

    /// Draws up to `amount` joules this tick. Void sources always deliver in full.
    pub fn draw(&mut self, db: &PrototypeDb, source: &EnergySource, amount: Energy) -> Energy {
        match (self, source) {
            (EnergyState::Burner(b), EnergySource::Burner { effectivity, .. }) => b.consume(db, amount, *effectivity),
            (EnergyState::Electric { buffer }, _) => {
                let take = amount.min(*buffer);
                *buffer -= take;
                take
            }
            (_, EnergySource::Void) => amount,
            _ => Fixed::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effectivity_scales_fuel_use() {
        // 4 MJ of fuel at 50% effectivity yields 2 MJ.
        let mut b = Burner::new(1);
        b.remaining = Fixed::from_int(4_000_000);
        let db = PrototypeDb::default();
        let got = b.consume(&db, Fixed::from_int(3_000_000), Fixed::from_ratio(1, 2));
        assert_eq!(got, Fixed::from_int(2_000_000));
        assert_eq!(b.remaining, Fixed::ZERO);
    }
}
