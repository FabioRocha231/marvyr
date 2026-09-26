//! v39: pesca. Parado no mar, Espaço lança a linha; entre 3 e 8 s depois o
//! peixe morde e o capitão tem uma janela curta para puxar. O tempo é todo
//! do servidor — o client só mostra a boia e manda o `CastLine`. Peixe é
//! recurso bruto de jogador (coleta): Peixe comum e o raro Peixe-Lanterna,
//! que entra nos frascos.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_items::{ItemDefinition, ItemInstance, ItemKind};
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::RiskTier;
use marvyr_protocol::{ActionKind, CastLine};
use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};
use tracing::info;

use crate::net::{DevItems, ServerShip, ServerWorldMap};
use crate::seafaring::send_action;
use crate::sets::SimulationSet;

pub const FISH: &str = "Peixe";
pub const LANTERN_FISH: &str = "Peixe-Lanterna";
/// Mais rápido que isto a linha não fica na água (m/s).
const MAX_SPEED: f32 = 1.5;
/// Mordida entre estes segundos depois do lançamento.
const BITE: (f32, f32) = (3.0, 8.0);
/// Janela para puxar depois da mordida (s), com folga de rede.
pub const WINDOW: f32 = 1.5;
/// Renome por peixe fisgado (e o Diário conta pelo motivo).
pub const RENOWN_PER_FISH: u32 = 4;
pub const REASON: &str = "pesca";

pub fn fish_id() -> ItemDefinitionId {
    ItemDefinitionId::stable(FISH)
}

pub fn lantern_id() -> ItemDefinitionId {
    ItemDefinitionId::stable(LANTERN_FISH)
}

pub fn definitions() -> [ItemDefinition; 2] {
    let fish = |id, name: &str, max_stack| ItemDefinition {
        id,
        kind: ItemKind::Resource,
        equipment: None,
        max_stack,
        base_weight: 1,
        tags: Default::default(),
        display_name: String::from(name),
    };
    [
        fish(fish_id(), FISH, 50),
        fish(lantern_id(), LANTERN_FISH, 20),
    ]
}

#[derive(Debug, Clone, Copy)]
struct Line {
    bite_at: f32,
    bitten: bool,
}

#[derive(Resource, Default)]
pub struct FishingLines(HashMap<u32, Line>);

pub fn install(app: &mut App) {
    app.init_resource::<FishingLines>().add_systems(
        FixedUpdate,
        (
            handle_cast_line.in_set(SimulationSet::Input),
            tick_lines.in_set(SimulationSet::EconomyConsequences),
        ),
    );
}

fn unit() -> f32 {
    (crate::seafaring::roll() >> 104) as f32 / (1u32 << 24) as f32
}

/// O que veio no anzol: mar sem lei dá mais Peixe-Lanterna.
fn catch(tier: Option<RiskTier>, pick: f32, size: f32) -> (ItemDefinitionId, u32) {
    let rare_chance = match tier {
        Some(RiskTier::Lawless) => 0.3,
        Some(RiskTier::Frontier) => 0.15,
        _ => 0.05,
    };
    if pick < rare_chance {
        (lantern_id(), 1)
    } else {
        (fish_id(), 2 + (size * 3.0) as u32)
    }
}

/// Espaço: lança; mordendo, puxa; cedo demais, espanta.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn handle_cast_line(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<CastLine>>,
    mut lines: ResMut<FishingLines>,
    mut ships: Query<&mut ServerShip>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let now = time.elapsed_secs();
    for event in events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if ship.presence != VesselPresence::AtSea {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Fish,
                false,
                "Pescar só no mar.",
            );
            continue;
        }
        let Some(line) = lines.0.remove(&ship.ship_id) else {
            if ship.motion.speed.abs() > MAX_SPEED {
                send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Fish,
                    false,
                    "Recolha as velas para pescar.",
                );
                continue;
            }
            let bite_at = now + BITE.0 + (BITE.1 - BITE.0) * unit();
            lines.0.insert(
                ship.ship_id,
                Line {
                    bite_at,
                    bitten: false,
                },
            );
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::FishCast,
                true,
                "Linha na água...",
            );
            continue;
        };
        if now < line.bite_at {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Fish,
                false,
                "Cedo demais: o peixe fugiu.",
            );
            continue;
        }
        if now > line.bite_at + WINDOW {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Fish,
                false,
                "Tarde demais: o peixe escapou.",
            );
            continue;
        }
        let tier = map
            .0
            .zone_at(ship.motion.x, ship.motion.y)
            .ok()
            .map(|zone| zone.tier);
        let (item, quantity) = catch(tier, unit(), unit());
        let found = ItemInstance::new_resource(ItemInstanceId::new(), item, quantity);
        if ship.hold.insert(&dev.catalog, found).is_err() {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Fish,
                false,
                "Porão cheio: o peixe volta para o mar.",
            );
            continue;
        }
        renown.send(crate::renown::RenownEarned {
            character: ship.character,
            amount: RENOWN_PER_FISH * quantity,
            reason: REASON,
        });
        let name = if item == lantern_id() {
            LANTERN_FISH
        } else {
            FISH
        };
        info!(ship_id = ship.ship_id, quantity, name, "peixe fisgado");
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Fish,
            true,
            format!("Fisgou {quantity} {name}!"),
        );
    }
}

/// Mordida na hora certa; linha recolhida se o navio anda, atraca ou some;
/// peixe que mordeu e não foi puxado escapa.
fn tick_lines(
    time: Res<Time>,
    mut lines: ResMut<FishingLines>,
    ships: Query<&ServerShip>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let now = time.elapsed_secs();
    lines.0.retain(|ship_id, line| {
        let Some(ship) = ships.iter().find(|s| s.ship_id == *ship_id) else {
            return false;
        };
        let Some(client_id) = ship.client_id else {
            return false;
        };
        if ship.presence != VesselPresence::AtSea || ship.motion.speed.abs() > MAX_SPEED {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Fish,
                false,
                "Linha recolhida.",
            );
            return false;
        }
        if !line.bitten && now >= line.bite_at {
            line.bitten = true;
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::FishBite,
                true,
                "Mordeu! Puxe (Espaço)!",
            );
        }
        if now > line.bite_at + WINDOW + 0.5 {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Fish,
                false,
                "O peixe escapou.",
            );
            return false;
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lawless_water_hooks_more_lantern_fish() {
        let rare = |tier| {
            (0..100)
                .filter(|i| catch(tier, *i as f32 / 100.0, 0.5).0 == lantern_id())
                .count()
        };
        assert!(rare(Some(RiskTier::Lawless)) > rare(Some(RiskTier::Frontier)));
        assert!(rare(Some(RiskTier::Frontier)) > rare(Some(RiskTier::Protected)));
        assert_eq!(catch(None, 0.99, 0.0), (fish_id(), 2));
        assert_eq!(catch(None, 0.99, 0.99), (fish_id(), 4));
    }

    #[test]
    fn fish_are_raw_resources() {
        for definition in definitions() {
            assert_eq!(definition.kind, ItemKind::Resource);
            assert!(definition.equipment.is_none());
        }
    }
}
