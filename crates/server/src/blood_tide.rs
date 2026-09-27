//! v32: Maré Sangrenta (Helltide do D4). Enquanto o diretor mantém o
//! evento ativo, o mar sem lei ferve:
//!
//! - **Ondas**: Saqueadores da Maré (sempre elite) chegam cada vez mais
//!   rápido (40 s no começo, 20 s no fim), até 5 vivos.
//! - **Cinza Sangrenta**: todo NPC afundado dentro da maré deixa cinza no
//!   destroço (recurso bruto, Pilar 1).
//! - **Baús Malditos**: encoste com 10 cinzas no porão e o baú suga as
//!   cinzas e paga recurso bruto de alto risco. Sem intent novo: abrir é
//!   chegar perto (o servidor decide).
//!
//! Quando a maré baixa, os saqueadores afundam com ela e os baús fechados
//! somem.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_items::ItemInstance;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::SeaEventKind;
use marvyr_protocol::{ActionKind, WorldEventKind};
use marvyr_shared::ids::ItemInstanceId;
use tracing::info;

use crate::net::{DevItems, ReliableChannel, ServerShip, ServerWorldMap};
use crate::npc::{NpcRole, NpcShip};
use crate::seafaring::ServerSeaEvents;
use crate::sets::SimulationSet;

/// Cinzas que um baú suga.
pub const ASH_PER_CHEST: u32 = 10;
/// Cinza por afundamento dentro da maré (Saqueador rende mais).
const ASH_PER_KILL: u32 = 3;
const ASH_PER_REAVER: u32 = 5;
/// Alcance (m) do casco até o baú.
const CHEST_REACH: f32 = 45.0;
const CHEST_COUNT: usize = 3;
const MAX_REAVERS: usize = 5;
/// Intervalo entre ondas: começa folgado e aperta até o fim da maré.
const WAVE_SECS: (f32, f32) = (40.0, 20.0);
/// Primeira onda depois que a maré sobe.
const FIRST_WAVE: f32 = 8.0;
/// Recusa do baú (texto fixo: o client traduz pela frase inteira).
const CHEST_REFUSAL: &str = "O Baú Maldito pede 10 Cinzas Sangrentas: afunde navios na maré.";
/// Um aviso de "faltam cinzas" por navio a cada tanto.
const REFUSAL_SECS: f32 = 6.0;

/// Estado da maré em curso (vazio quando não há maré).
#[derive(Resource, Default)]
pub struct BloodTide {
    event_id: Option<u32>,
    /// (x, y, raio) da área.
    area: Option<(f32, f32, f32)>,
    /// Baús ainda fechados (vão pelo fio no `SeaEventState`).
    pub chests: Vec<(f32, f32)>,
    wave_in: f32,
    waves: u32,
    /// Saqueadores que esta maré soltou. Só esses contam e afundam com
    /// ela: caçadores da Carga Amaldiçoada e ondas do Abismo também são
    /// `Reaver`, e sumir com eles dava a camada do Abismo de graça.
    reavers: Vec<u32>,
    refusals: HashMap<u32, f32>,
}

impl BloodTide {
    /// Cinza que um afundamento em `at` deixa (nenhuma fora da maré).
    pub fn ash_for(&self, role: NpcRole, at: (f32, f32)) -> Option<u32> {
        let (x, y, radius) = self.area?;
        let inside = Vec2::new(at.0 - x, at.1 - y).length() <= radius;
        inside.then_some(if role == NpcRole::Reaver {
            ASH_PER_REAVER
        } else {
            ASH_PER_KILL
        })
    }
}

/// Intervalo da próxima onda pelo que falta da maré (`left` de 1 a 0).
/// v35: Renome do Baú Maldito (e o Diário conta o baú por ele).
const CHEST_RENOWN: u32 = 25;

pub fn wave_secs(left: f32) -> f32 {
    WAVE_SECS.1 + (WAVE_SECS.0 - WAVE_SECS.1) * left.clamp(0.0, 1.0)
}

/// Recompensa do baú: recurso bruto de alto risco (Pilar 1).
fn chest_reward(dev: &DevItems) -> [(marvyr_shared::ids::ItemDefinitionId, u32); 3] {
    [
        (dev.abyssal_pearl, 4),
        (dev.abyssal_amber, 3),
        (dev.coral, 6),
    ]
}

pub fn install(app: &mut App) {
    app.init_resource::<BloodTide>();
    app.add_systems(
        FixedUpdate,
        run_blood_tide.in_set(SimulationSet::EconomyConsequences),
    );
}

#[allow(clippy::too_many_arguments)]
fn run_blood_tide(
    mut commands: Commands,
    time: Res<Time>,
    events: Res<ServerSeaEvents>,
    mut tide: ResMut<BloodTide>,
    mut connection_manager: ResMut<ConnectionManager>,
    (dev, dev_ships, map, config): (
        Res<DevItems>,
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    (mut npc_ids, mut wreck_ids, mut live_wrecks): (
        ResMut<crate::npc::NpcIdCounter>,
        ResMut<crate::net::WreckIdCounter>,
        ResMut<crate::net::LiveWreckRecords>,
    ),
    npcs: Query<(Entity, &NpcShip)>,
    mut ships: Query<&mut ServerShip>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
) {
    let active = events
        .director
        .active()
        .filter(|event| event.kind == SeaEventKind::BloodTide)
        .copied();
    let Some(event) = active else {
        // A maré baixou: saqueadores afundam com ela, baús fechados somem.
        if tide.event_id.take().is_some() {
            for (entity, npc) in &npcs {
                if tide.reavers.contains(&npc.ship_id) {
                    commands.entity(entity).despawn();
                }
            }
            *tide = BloodTide::default();
            info!("maré sangrenta baixou");
        }
        return;
    };
    let center = Vec2::new(event.x, event.y);
    if tide.event_id != Some(event.id) {
        let chests = (0..CHEST_COUNT)
            .map(|i| {
                let angle = i as f32 / CHEST_COUNT as f32 * std::f32::consts::TAU + 0.4;
                let want = center + Vec2::from_angle(angle) * event.radius * 0.55;
                let at = crate::seafaring::water_near(&map.0, center, want);
                (at.x, at.y)
            })
            .collect();
        *tide = BloodTide {
            event_id: Some(event.id),
            area: Some((event.x, event.y, event.radius)),
            chests,
            wave_in: FIRST_WAVE,
            ..default()
        };
        info!(x = event.x, y = event.y, "maré sangrenta subiu");
    }
    let dt = time.delta_secs();

    // Ondas: a ameaça cresce conforme a maré avança.
    let alive: Vec<u32> = npcs
        .iter()
        .map(|(_, npc)| npc.ship_id)
        .filter(|id| tide.reavers.contains(id))
        .collect();
    tide.reavers = alive;
    tide.wave_in -= dt;
    if tide.wave_in <= 0.0 {
        tide.wave_in = wave_secs(event.remaining / SeaEventKind::BloodTide.duration());
        if tide.reavers.len() < MAX_REAVERS {
            tide.waves += 1;
            let angle = tide.waves as f32 * 2.399_963;
            let want = center + Vec2::from_angle(angle) * event.radius * 0.7;
            let at = crate::seafaring::water_near(&map.0, center, want);
            let (id, npc) = crate::npc::build_npc(
                &dev_ships,
                &map.0,
                &config,
                &mut npc_ids,
                NpcRole::Reaver,
                (at.x, at.y),
            );
            info!(
                npc_id = id,
                elite = npc.elite,
                wave = tide.waves,
                "saqueador da maré"
            );
            commands.spawn((npc,));
            tide.reavers.push(id);
        }
    }

    // Baús: encostou com cinza suficiente, abre.
    for cooldown in tide.refusals.values_mut() {
        *cooldown -= dt;
    }
    tide.refusals.retain(|_, cooldown| *cooldown > 0.0);
    let now = time.elapsed_secs();
    for mut ship in &mut ships {
        if ship.presence != VesselPresence::AtSea {
            continue;
        }
        let Some(client_id) = ship.client_id else {
            continue;
        };
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        let Some(index) = tide
            .chests
            .iter()
            .position(|(x, y)| at.distance(Vec2::new(*x, *y)) <= CHEST_REACH)
        else {
            continue;
        };
        if ship.hold.remove(dev.blood_ash, ASH_PER_CHEST).is_err() {
            if let std::collections::hash_map::Entry::Vacant(slot) =
                tide.refusals.entry(ship.ship_id)
            {
                slot.insert(REFUSAL_SECS);
                crate::seafaring::send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::CursedChest,
                    false,
                    CHEST_REFUSAL,
                );
            }
            continue;
        }
        let chest = tide.chests.remove(index);
        // Porão cheio: o que não coube boia num destroço só dele.
        let mut overflow = Vec::new();
        for (item, quantity) in chest_reward(&dev) {
            let found = ItemInstance::new_resource(ItemInstanceId::new(), item, quantity);
            if ship.hold.insert(&dev.catalog, found).is_err() {
                overflow.push((item, quantity));
            }
        }
        let spilled = !overflow.is_empty();
        if spilled {
            crate::npc::spawn_spoils_wreck(
                &mut commands,
                &mut wreck_ids,
                &mut live_wrecks,
                overflow,
                Some(ship.character),
                chest,
                now,
            );
        }
        info!(ship_id = ship.ship_id, spilled, "baú maldito aberto");
        renown.send(crate::renown::RenownEarned {
            character: ship.character,
            amount: CHEST_RENOWN,
            reason: "Baú Maldito",
        });
        crate::seafaring::send_action(
            &mut connection_manager,
            client_id,
            ActionKind::CursedChest,
            true,
            if spilled {
                "Baú Maldito aberto! Porão cheio: o resto boia ao lado."
            } else {
                "Baú Maldito aberto! Pérolas e âmbar no porão."
            },
        );
        let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
            &marvyr_protocol::WorldEvent {
                text: String::from("Um Baú Maldito se abriu na Maré Sangrenta!"),
                kind: WorldEventKind::Alert,
            },
            NetworkTarget::All,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_text_names_the_real_price() {
        assert!(CHEST_REFUSAL.contains(&format!("{ASH_PER_CHEST} Cinzas")));
    }

    #[test]
    fn waves_tighten_as_the_tide_runs_out() {
        assert_eq!(wave_secs(1.0), WAVE_SECS.0);
        assert_eq!(wave_secs(0.0), WAVE_SECS.1);
        assert!(wave_secs(0.5) < wave_secs(0.9));
    }

    #[test]
    fn ash_only_falls_inside_the_tide() {
        let mut tide = BloodTide::default();
        assert_eq!(tide.ash_for(NpcRole::Pirate, (0.0, 0.0)), None);
        tide.area = Some((100.0, 0.0, 50.0));
        assert_eq!(
            tide.ash_for(NpcRole::Pirate, (120.0, 0.0)),
            Some(ASH_PER_KILL)
        );
        assert_eq!(
            tide.ash_for(NpcRole::Reaver, (100.0, 40.0)),
            Some(ASH_PER_REAVER)
        );
        assert_eq!(tide.ash_for(NpcRole::Pirate, (200.0, 0.0)), None);
    }
}
