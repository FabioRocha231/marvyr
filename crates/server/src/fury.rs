//! v36: Fúria do Mar. Cada navio afundado em seguida, sem voltar ao porto,
//! sobe a fúria do capitão e o butim bruto do próximo destroço (+10% por
//! ponto, até o dobro). Atracar ou perder o casco zera — o "só mais um"
//! tem preço.

use std::collections::HashMap;

use bevy::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_shared::ids::CharacterId;

use crate::net::ServerShip;
use crate::sets::SimulationSet;

pub const MAX_FURY: u8 = 10;
/// Butim a mais por ponto de fúria (%).
pub use marvyr_protocol::fury::PER_POINT_PCT;

#[derive(Resource, Default)]
pub struct SeaFury {
    fury: HashMap<CharacterId, u8>,
    /// Casco em que a fúria foi ganha: casco novo (naufrágio) zera.
    hull: HashMap<CharacterId, u32>,
}

impl SeaFury {
    pub fn get(&self, character: CharacterId) -> u8 {
        self.fury.get(&character).copied().unwrap_or(0)
    }

    /// Butim bruto com a fúria de agora.
    pub fn spoils(&self, character: CharacterId, base: u32) -> u32 {
        base * (100 + PER_POINT_PCT * u32::from(self.get(character))) / 100
    }

    /// Mais um no fundo: sobe a fúria (até o teto) e devolve a nova.
    pub fn bump(&mut self, character: CharacterId) -> u8 {
        let fury = self.fury.entry(character).or_default();
        *fury = (*fury + 1).min(MAX_FURY);
        *fury
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<SeaFury>().add_systems(
        FixedUpdate,
        cool_down.in_set(SimulationSet::EconomyConsequences),
    );
}

/// Atracou ou trocou de casco (afundou e renasceu): a fúria acaba.
fn cool_down(ships: Query<&ServerShip>, mut fury: ResMut<SeaFury>) {
    // Quem saiu do mar (logout, fim da janela de graça) sai dos mapas:
    // eles não crescem com cada capitão que já entrou desde o boot.
    let online: std::collections::HashSet<CharacterId> = ships
        .iter()
        .filter(|ship| ship.client_id.is_some())
        .map(|ship| ship.character)
        .collect();
    fury.fury.retain(|character, _| online.contains(character));
    fury.hull.retain(|character, _| online.contains(character));
    for ship in &ships {
        if ship.client_id.is_none() {
            continue;
        }
        let new_hull = fury.hull.insert(ship.character, ship.ship_id) != Some(ship.ship_id);
        if new_hull || matches!(ship.presence, VesselPresence::Docked(_)) {
            fury.fury.remove(&ship.character);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fury_climbs_to_double_loot_and_caps() {
        let mut fury = SeaFury::default();
        let captain = CharacterId::new();
        assert_eq!(fury.spoils(captain, 10), 10);
        fury.bump(captain);
        fury.bump(captain);
        assert_eq!(fury.spoils(captain, 10), 12);
        for _ in 0..20 {
            fury.bump(captain);
        }
        assert_eq!(fury.get(captain), MAX_FURY);
        assert_eq!(fury.spoils(captain, 10), 20, "teto: o dobro");
    }
}
