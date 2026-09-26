//! v34: chefe de mundo agendado (os world bosses do D4). O Leviatã emerge
//! em horário fixo, num dos sítios fundos do mar sem lei, com aviso de
//! dois minutos para o servidor inteiro convergir. Corre em paralelo ao
//! diretor de eventos.
//!
//! Cada capitão que tirou pelo menos 2% do casco dele ganha o próprio baú
//! (destroço exclusivo, só recurso bruto — Pilar 1) e Renome. Se ninguém o
//! afundar em dez minutos, ele volta ao abismo.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_protocol::{SeaEventState, WorldEventKind};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId};
use tracing::info;

use crate::net::{DevItems, ReliableChannel, ServerWorldMap};
use crate::npc::{NpcRole, NpcShip, LEVIATHAN_HP};
use crate::sets::SimulationSet;

/// Primeiro Leviatã depois do boot e intervalo entre eles (s).
const FIRST_SPAWN: f32 = 600.0;
const INTERVAL: f32 = 1_500.0;
/// Aviso antes de emergir (s).
const WARNING: f32 = 120.0;
/// Quanto tempo ele fica antes de voltar ao fundo (s).
const LIFETIME: f32 = 600.0;
/// Dano mínimo para ter parte no butim (2% do casco).
pub const SHARE_MIN_DAMAGE: u32 = LEVIATHAN_HP / 50;
/// Renome de cada participante.
const RENOWN_SHARE: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Contando até emergir (`in_secs`); `warned` = aviso já saiu.
    Waiting { in_secs: f32, warned: bool },
    /// No mar: o NPC e quanto falta para ele ir embora.
    Alive { npc_id: u32, leaves_in: f32 },
}

#[derive(Resource)]
pub struct WorldBoss {
    phase: Phase,
    cycle: u32,
    site: (f32, f32),
    /// Dano por capitão neste Leviatã.
    damage: HashMap<CharacterId, u32>,
    /// Onde afundou (o `simulate_npcs` marca; aqui paga).
    slain_at: Option<(f32, f32)>,
}

impl Default for WorldBoss {
    fn default() -> Self {
        // Dev: MARVYR_BOSS_IN=<s> antecipa o primeiro.
        let first = std::env::var("MARVYR_BOSS_IN")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(FIRST_SPAWN);
        Self {
            phase: Phase::Waiting {
                in_secs: first,
                warned: false,
            },
            cycle: 0,
            site: (0.0, 0.0),
            damage: HashMap::new(),
            slain_at: None,
        }
    }
}

impl WorldBoss {
    /// Golpe de um capitão no Leviatã (chamado pelo `simulate_npcs`).
    pub fn record_hit(&mut self, character: CharacterId, damage: u32) {
        *self.damage.entry(character).or_default() += damage;
    }

    /// Dev/teste: emerge no próximo tick.
    pub fn summon_now(&mut self) {
        self.phase = Phase::Waiting {
            in_secs: 0.0,
            warned: true,
        };
    }

    pub fn slain(&mut self, at: (f32, f32)) {
        self.slain_at = Some(at);
    }

    /// Quem leva parte do butim: tirou pelo menos `SHARE_MIN_DAMAGE`.
    pub fn sharers(&self) -> Vec<CharacterId> {
        let mut out: Vec<CharacterId> = self
            .damage
            .iter()
            .filter(|(_, damage)| **damage >= SHARE_MIN_DAMAGE)
            .map(|(character, _)| *character)
            .collect();
        out.sort_by_key(|c| c.0);
        out
    }

    /// Linha do HUD de eventos: contagem até emergir, depois o tempo que
    /// ele ainda fica (a área segue o monstro).
    pub fn wire(&self, npcs: &Query<&NpcShip>) -> Option<SeaEventState> {
        let (x, y) = self.site;
        match self.phase {
            Phase::Waiting { in_secs, .. } if in_secs <= WARNING => Some(SeaEventState {
                event_id: u32::MAX - self.cycle,
                kind: marvyr_protocol::SeaEventKind::WorldBoss,
                name: String::from("Leviatã emerge"),
                x,
                y,
                radius: 220.0,
                remaining_secs: in_secs.max(0.0),
                chests: Vec::new(),
            }),
            Phase::Waiting { .. } => None,
            Phase::Alive { npc_id, leaves_in } => {
                let (x, y) = npcs
                    .iter()
                    .find(|npc| npc.ship_id == npc_id)
                    .map_or((x, y), |npc| (npc.motion.x, npc.motion.y));
                Some(SeaEventState {
                    event_id: u32::MAX - self.cycle,
                    kind: marvyr_protocol::SeaEventKind::WorldBoss,
                    name: String::from("Leviatã"),
                    x,
                    y,
                    radius: 220.0,
                    remaining_secs: leaves_in.max(0.0),
                    chests: Vec::new(),
                })
            }
        }
    }
}

/// Baú de cada participante: bruto de alto risco (Pilar 1).
fn share(dev: &DevItems) -> Vec<(ItemDefinitionId, u32)> {
    vec![
        (dev.abyssal_pearl, 3),
        (dev.abyssal_amber, 3),
        (dev.coral, 8),
        (dev.blood_ash, 5),
    ]
}

pub fn install(app: &mut App) {
    app.init_resource::<WorldBoss>();
    app.add_systems(
        FixedUpdate,
        run_world_boss.in_set(SimulationSet::EconomyConsequences),
    );
}

fn announce(connection_manager: &mut ConnectionManager, text: String) {
    let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
        &marvyr_protocol::WorldEvent {
            text,
            kind: WorldEventKind::Alert,
        },
        NetworkTarget::All,
    );
}

#[allow(clippy::too_many_arguments)]
fn run_world_boss(
    mut commands: Commands,
    time: Res<Time>,
    mut boss: ResMut<WorldBoss>,
    mut connection_manager: ResMut<ConnectionManager>,
    (dev, dev_ships, map, config): (
        Res<DevItems>,
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    (mut npc_ids, mut wreck_ids, mut live_wrecks, mut renown): (
        ResMut<crate::npc::NpcIdCounter>,
        ResMut<crate::net::WreckIdCounter>,
        ResMut<crate::net::LiveWreckRecords>,
        EventWriter<crate::renown::RenownEarned>,
    ),
    npcs: Query<(Entity, &NpcShip)>,
) {
    let dt = time.delta_secs();
    // Afundou: cada participante ganha o próprio baú.
    if let Some(at) = boss.slain_at.take() {
        let sharers = boss.sharers();
        let now = time.elapsed_secs();
        for (i, character) in sharers.iter().enumerate() {
            let angle = i as f32 * 2.399_963;
            let spot = Vec2::new(at.0, at.1) + Vec2::from_angle(angle) * (18.0 + 6.0 * i as f32);
            crate::npc::spawn_spoils_wreck(
                &mut commands,
                &mut wreck_ids,
                &mut live_wrecks,
                share(&dev),
                Some(*character),
                (spot.x, spot.y),
                now,
            );
            renown.send(crate::renown::RenownEarned {
                character: *character,
                amount: RENOWN_SHARE,
                reason: "Leviatã afundado",
            });
        }
        info!(sharers = sharers.len(), "Leviatã afundado; butim dividido");
        announce(
            &mut connection_manager,
            format!(
                "O Leviatã afundou! {} capitães dividem o butim.",
                sharers.len()
            ),
        );
        boss.damage.clear();
        boss.phase = Phase::Waiting {
            in_secs: INTERVAL,
            warned: false,
        };
        return;
    }
    match boss.phase {
        Phase::Waiting { in_secs, warned } => {
            let sites = &map.0.features().kraken_sites;
            if sites.is_empty() {
                return;
            }
            boss.site = sites[boss.cycle as usize % sites.len()];
            let in_secs = in_secs - dt;
            let mut warned = warned;
            if !warned && in_secs <= WARNING {
                warned = true;
                announce(
                    &mut connection_manager,
                    String::from(
                        "O mar treme: o Leviatã emerge em 2 minutos! Todo capitão que lutar leva parte.",
                    ),
                );
            }
            if in_secs > 0.0 {
                boss.phase = Phase::Waiting { in_secs, warned };
                return;
            }
            let (npc_id, npc) = crate::npc::build_npc(
                &dev_ships,
                &map.0,
                &config,
                &mut npc_ids,
                NpcRole::Leviathan,
                boss.site,
            );
            commands.spawn((npc,));
            boss.cycle += 1;
            boss.damage.clear();
            boss.phase = Phase::Alive {
                npc_id,
                leaves_in: LIFETIME,
            };
            info!(npc_id, x = boss.site.0, y = boss.site.1, "Leviatã emergiu");
            announce(
                &mut connection_manager,
                String::from("O Leviatã emergiu no mar sem lei!"),
            );
        }
        Phase::Alive { npc_id, leaves_in } => {
            let leaves_in = leaves_in - dt;
            let alive = npcs.iter().find(|(_, npc)| npc.ship_id == npc_id);
            if leaves_in > 0.0 && alive.is_some() {
                boss.phase = Phase::Alive { npc_id, leaves_in };
                return;
            }
            // Sumiu sem `slain` (despawn por fora) ou deu a hora: volta ao
            // fundo sem butim.
            if let Some((entity, _)) = alive {
                commands.entity(entity).despawn();
                announce(
                    &mut connection_manager,
                    String::from("O Leviatã voltou ao abismo."),
                );
            }
            boss.damage.clear();
            boss.phase = Phase::Waiting {
                in_secs: INTERVAL,
                warned: false,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_fighters_share_the_loot() {
        let mut boss = WorldBoss::default();
        let (a, b, c) = (CharacterId::new(), CharacterId::new(), CharacterId::new());
        boss.record_hit(a, SHARE_MIN_DAMAGE);
        boss.record_hit(b, SHARE_MIN_DAMAGE / 2);
        boss.record_hit(b, SHARE_MIN_DAMAGE / 2);
        boss.record_hit(c, 1);
        let mut expected = vec![a, b];
        expected.sort_by_key(|c| c.0);
        assert_eq!(boss.sharers(), expected, "golpe somado conta; beliscão não");
    }
}
