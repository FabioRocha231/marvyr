//! Combate ativo (v54): o capitão mira com o mouse (salva do bordo que
//! encara a mira), solta o leque, larga barril na esteira e abalroa. O
//! dano segue o caminho de sempre — bala vira `ServerProjectile`, choque
//! vira `Impact` (jogador) ou `NpcRams` (NPC) — então zona, notoriedade,
//! Renome e despojos continuam decididos num lugar só.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_combat::active::{self, RAM_BOOST, RAM_DAMAGE, RAM_REACH};
use marvyr_domain_combat::{aim_at, CombatActionKind};
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{ActionKind, CombatAction};

use crate::gunnery::ContactKind;
use crate::gunnery::{auto_hostile, cannons_cold, contacts, reload_secs, salvo, weapon_of};
use crate::net::{
    CombatTuning, DeferredImpacts, Impact, ProjectileIdCounter, ServerProjectile, ServerRiskPolicy,
    ServerShip, ServerWorldMap,
};
use crate::npc::NpcShip;
use crate::reputation::Reputation;
use crate::seafaring::send_action;

/// Choques de abalroar em NPC, para o `simulate_npcs` aplicar:
/// (id do NPC, navio atacante, dano).
#[derive(Resource, Default)]
pub struct NpcRams(pub Vec<(u32, u32, u32)>);

pub fn install(app: &mut App) {
    app.init_resource::<NpcRams>();
}

#[allow(clippy::too_many_arguments)]
pub fn handle_combat_actions(
    mut commands: Commands,
    mut connection_manager: ResMut<ConnectionManager>,
    mut actions: EventReader<ServerReceiveMessage<CombatAction>>,
    tuning: Res<CombatTuning>,
    map: Res<ServerWorldMap>,
    risk: Res<ServerRiskPolicy>,
    mut projectile_ids: ResMut<ProjectileIdCounter>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in actions.read() {
        let client_id = event.from();
        let action = *event.message();
        // Fronteira de confiança: mira vem do client.
        if !(action.aim_x.is_finite() && action.aim_y.is_finite()) {
            continue;
        }
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if matches!(ship.presence, VesselPresence::Docked(_)) {
            continue;
        }
        // v57: gemas encaixadas no navio mudam o que cada skill faz.
        let variants = skill_variants(&ship);
        // Abalroar também é esquiva: vale em qualquer água (o choque só
        // machuca onde a vítima pode ser atacada).
        if action.kind == CombatActionKind::Ram {
            if ship.combat.try_ram(variants.long_ram) {
                let burst = ship.stats.speed * RAM_BOOST;
                ship.motion.speed = ship.motion.speed.max(burst);
            }
            continue;
        }
        if cannons_cold(&ship, &map, &risk) {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Gunnery,
                false,
                REFUSED_CROWN_FIRE,
            );
            continue;
        }
        let weapon = weapon_of(&ship, &tuning);
        let me = (ship.motion.x, ship.motion.y);
        let aim = (action.aim_x, action.aim_y);
        let shots = match action.kind {
            CombatActionKind::Broadside => {
                let (side, correction) = aim_at(ship.motion.heading, me, aim);
                let reload = reload_secs(&ship, &tuning);
                if !ship.battery.try_fire(side, reload) {
                    continue;
                }
                salvo(
                    &ship,
                    &mut projectile_ids,
                    &tuning,
                    weapon,
                    side,
                    correction,
                )
            }
            CombatActionKind::FanSalvo => {
                if !ship.combat.try_fan() {
                    continue;
                }
                let first = projectile_ids.0;
                projectile_ids.0 += active::fan_skill_ids(variants.fan);
                let heading = ship.motion.heading;
                active::fan_skill(
                    variants.fan,
                    first,
                    ship.ship_id,
                    me,
                    (
                        heading.cos() * ship.motion.speed,
                        heading.sin() * ship.motion.speed,
                    ),
                    (aim.1 - me.1).atan2(aim.0 - me.0),
                    // v58: a bombarda nasceu para o leque.
                    active::scaled(weapon, ship.kind.fan_bonus()),
                )
            }
            CombatActionKind::FireBarrel => {
                if !ship.combat.try_barrel() {
                    continue;
                }
                let first = projectile_ids.0;
                projectile_ids.0 += 3;
                active::barrel_skill(
                    variants.barrel,
                    first,
                    ship.ship_id,
                    me,
                    ship.motion.heading,
                    weapon,
                )
            }
            CombatActionKind::Ram => unreachable!("tratado acima"),
        };
        ship.black_flag.fired();
        ship.combat.mark_manual();
        info!(ship_id = ship.ship_id, kind = ?action.kind, "tiro na mão");
        commands.spawn_batch(shots.into_iter().map(|p| (ServerProjectile(p),)));
    }
}

/// v57: variantes de skill pelas gemas de todas as peças instaladas.
pub fn skill_variants(ship: &ServerShip) -> active::SkillVariants {
    active::SkillVariants::from_gems(
        ship.loadout
            .items()
            .flat_map(|custody| custody.instance.gems().iter().copied()),
    )
}

/// Texto da recusa de tiro em águas da coroa.
pub const REFUSED_CROWN_FIRE: &str = "Águas protegidas: a coroa não deixa abrir fogo";

/// Avança recargas e aplica o choque de quem está abalroando.
#[allow(clippy::too_many_arguments)]
pub fn tick_active_combat(
    time: Res<Time>,
    tuning: Res<CombatTuning>,
    reputation: Res<Reputation>,
    mut rams: ResMut<NpcRams>,
    mut deferred: ResMut<DeferredImpacts>,
    mut ships: Query<&mut ServerShip>,
    npcs: Query<&NpcShip>,
    parties: Res<crate::party::Parties>,
) {
    let dt = time.delta_secs();
    let seen = contacts(&ships, &npcs);
    let reach = tuning.hit_radius * RAM_REACH;
    for mut ship in &mut ships {
        ship.combat.tick(dt);
        if !ship.combat.is_ramming() {
            continue;
        }
        let me = (ship.motion.x, ship.motion.y);
        let shooter = (ship.ship_id, ship.character);
        let raised = ship.black_flag.is_raised();
        let damage = active::scaled(weapon_of(&ship, &tuning), RAM_DAMAGE).damage;
        for contact in &seen {
            let close = (contact.at.0 - me.0).hypot(contact.at.1 - me.1) <= reach;
            // Choque em inocente só com a mesma decisão do tiro
            // automático (Bandeira Negra ou alvo que já é hostil).
            if !close
                || !auto_hostile(shooter, raised, contact, &reputation, &parties)
                || !ship.combat.ram_hit(contact.ship_id)
            {
                continue;
            }
            match contact.kind {
                ContactKind::Npc { .. } => rams.0.push((contact.ship_id, ship.ship_id, damage)),
                ContactKind::Player { .. } => deferred.0.push(Impact {
                    projectile: None,
                    target_ship_id: contact.ship_id,
                    hull_damage: damage,
                    attacker_ship_id: ship.ship_id,
                    sail_damage: 0.0,
                    at: contact.at,
                    boarded: false,
                }),
            }
            info!(
                ship_id = ship.ship_id,
                target = contact.ship_id,
                damage,
                "abalroou"
            );
        }
    }
}
