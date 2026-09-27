//! Tiro automático em 360° e Bandeira Negra (v21). O navio dispara sozinho
//! quando a recarga fecha, no alvo mais próximo que ele PODE atacar sem o
//! capitão ter decidido nada: NPC fora-da-lei, NPC que o caça, fora-da-lei
//! (Procurado ou Bandeira Negra) e quem o acertou há pouco. Atacar inocente
//! é decisão consciente: travar o alvo (Q) ou içar a Bandeira Negra (R),
//! que faz mirar em todo mundo — e todo mundo mirar nele.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_combat::{
    aim_at, nearest_in_range, BlackFlag, FlagRefusal, Projectile, WeaponParams,
};
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{ActionKind, LockTarget, SetBlackFlag};
use marvyr_shared::ids::CharacterId;

use crate::net::{
    CombatTuning, ProjectileIdCounter, ServerProjectile, ServerRiskPolicy, ServerShip,
    ServerWorldMap,
};
use crate::npc::{NpcRole, NpcShip, NpcState};
use crate::reputation::Reputation;
use crate::seafaring::send_action;

/// Alvo travado se solta além deste múltiplo do alcance.
const LOCK_KEEP: f32 = 2.0;

pub const RAISED: &str = "Içando a Bandeira Negra: em instantes todos são alvo, e você também";
pub const LOWERED: &str = "Bandeira Negra arriada";
pub const NOT_CALM: &str = "Disparou há pouco: a Bandeira Negra só desce após 60 s sem tiro";
pub const REFUSED_PORT: &str = "Atracado não se iça a Bandeira Negra";
pub const REFUSED_CROWN: &str = "Águas protegidas: a coroa não deixa içar a Bandeira Negra";
pub const FORCED_DOWN: &str = "Águas protegidas: a coroa mandou arriar a Bandeira Negra";
/// Atracar com a bandeira içada: o porto também arria (texto próprio).
pub const DOCKED_DOWN: &str = "No porto, a Bandeira Negra desce do mastro";
pub const LOCKED: &str = "Alvo travado: o tiro automático vai nele";
pub const UNLOCKED: &str = "Alvo solto";
pub const NO_TARGET: &str = "Nenhum navio no alcance para travar";

#[derive(Debug, Clone, Copy)]
pub struct Contact {
    pub ship_id: u32,
    pub at: (f32, f32),
    pub kind: ContactKind,
}

#[derive(Debug, Clone, Copy)]
pub enum ContactKind {
    Player {
        character: CharacterId,
    },
    /// `hunting` = navio que este NPC persegue ou ataca agora.
    Npc {
        role: NpcRole,
        hunting: Option<u32>,
    },
}

/// O tiro automático de `shooter` mira `contact` sem o capitão decidir?
pub fn auto_hostile(
    shooter: (u32, CharacterId),
    flag_raised: bool,
    contact: &Contact,
    reputation: &Reputation,
    parties: &crate::party::Parties,
) -> bool {
    if contact.ship_id == shooter.0 {
        return false;
    }
    // v55: companheiro de party nunca é alvo, nem de Bandeira Negra.
    if let ContactKind::Player { character } = contact.kind {
        if parties.same_party(character, shooter.1) {
            return false;
        }
    }
    if flag_raised {
        return true;
    }
    match contact.kind {
        ContactKind::Player { character } => {
            character != shooter.1
                && (reputation.is_outlaw(character) || reputation.attacked_me(shooter.1, character))
        }
        ContactKind::Npc { role, hunting } => {
            matches!(
                role,
                NpcRole::Pirate
                    | NpcRole::Sloop
                    | NpcRole::Fireship
                    | NpcRole::Gunner
                    | NpcRole::Mender
                    | NpcRole::Kraken
                    | NpcRole::Guardian
                    | NpcRole::Reaver
                    | NpcRole::Leviathan
            ) || hunting == Some(shooter.0)
        }
    }
}

pub(crate) fn contacts(ships: &Query<&mut ServerShip>, npcs: &Query<&NpcShip>) -> Vec<Contact> {
    ships
        .iter()
        .map(|ship| Contact {
            ship_id: ship.ship_id,
            at: (ship.motion.x, ship.motion.y),
            kind: ContactKind::Player {
                character: ship.character,
            },
        })
        .chain(
            npcs.iter()
                .filter(|npc| npc.ai.state != NpcState::Dead)
                .map(|npc| Contact {
                    ship_id: npc.ship_id,
                    at: (npc.motion.x, npc.motion.y),
                    kind: ContactKind::Npc {
                        role: npc.role,
                        hunting: matches!(npc.ai.state, NpcState::Chase { .. } | NpcState::Attack)
                            .then_some(npc.last_target)
                            .flatten(),
                    },
                }),
        )
        .collect()
}

/// Canhões frios: atracado, águas protegidas ou fora do mapa.
pub(crate) fn cannons_cold(
    ship: &ServerShip,
    map: &ServerWorldMap,
    risk: &ServerRiskPolicy,
) -> bool {
    matches!(ship.presence, VesselPresence::Docked(_))
        || !map
            .0
            .zone_at(ship.motion.x, ship.motion.y)
            .is_ok_and(|zone| risk.0.pvp_allowed(zone.tier))
}

pub(crate) fn weapon_of(ship: &ServerShip, tuning: &CombatTuning) -> WeaponParams {
    ship.ammo.load(WeaponParams {
        // v25: Frasco de Fúria engrossa a carga.
        damage: (ship.stats.weapon_damage as f32 * ship.flasks.damage_multiplier()).round() as u32,
        speed: tuning.projectile_speed,
        range: ship.stats.weapon_range,
        muzzle_offset: tuning.muzzle_offset,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn handle_gunnery_intents(
    mut connection_manager: ResMut<ConnectionManager>,
    mut flags: EventReader<ServerReceiveMessage<SetBlackFlag>>,
    mut locks: EventReader<ServerReceiveMessage<LockTarget>>,
    tuning: Res<CombatTuning>,
    map: Res<ServerWorldMap>,
    risk: Res<ServerRiskPolicy>,
    mut ships: Query<&mut ServerShip>,
    npcs: Query<&NpcShip>,
) {
    let seen = contacts(&ships, &npcs);
    for event in flags.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let (success, reason) = if event.message().raise {
            if matches!(ship.presence, VesselPresence::Docked(_)) {
                (false, REFUSED_PORT)
            } else if cannons_cold(&ship, &map, &risk) {
                (false, REFUSED_CROWN)
            } else {
                ship.black_flag.hoist();
                (true, RAISED)
            }
        } else {
            match ship.black_flag.lower() {
                Ok(()) => (true, LOWERED),
                Err(FlagRefusal::NotCalm { .. }) => (false, NOT_CALM),
            }
        };
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Gunnery,
            success,
            reason,
        );
    }
    for event in locks.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let (success, reason) = if ship.target_lock.take().is_some() {
            (true, UNLOCKED)
        } else {
            let me = (ship.motion.x, ship.motion.y);
            let own = ship.ship_id;
            ship.target_lock = nearest_in_range(
                me,
                weapon_of(&ship, &tuning).range,
                seen.iter()
                    .filter(|c| c.ship_id != own)
                    .map(|c| (c.ship_id, c.at)),
            )
            .map(|(id, _)| id);
            if ship.target_lock.is_some() {
                (true, LOCKED)
            } else {
                (false, NO_TARGET)
            }
        };
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Gunnery,
            success,
            reason,
        );
    }
}

/// Sobe/arria a bandeira com o tempo, arria à força em águas da coroa e
/// espelha quem está içado na `Reputation`.
pub fn tick_black_flags(
    time: Res<Time>,
    mut connection_manager: ResMut<ConnectionManager>,
    map: Res<ServerWorldMap>,
    risk: Res<ServerRiskPolicy>,
    mut reputation: ResMut<Reputation>,
    mut ships: Query<&mut ServerShip>,
) {
    let dt = time.delta_secs();
    for mut ship in &mut ships {
        if ship.black_flag != BlackFlag::Lowered {
            if cannons_cold(&ship, &map, &risk) {
                ship.black_flag.force_lower();
                if let Some(client_id) = ship.client_id {
                    send_action(
                        &mut connection_manager,
                        client_id,
                        ActionKind::Gunnery,
                        false,
                        if matches!(ship.presence, VesselPresence::Docked(_)) {
                            DOCKED_DOWN
                        } else {
                            FORCED_DOWN
                        },
                    );
                }
            } else {
                ship.black_flag.tick(dt);
            }
        }
        reputation.set_black_flag(ship.character, ship.black_flag.is_raised());
    }
}

/// Dispara sozinho no alvo travado ou no hostil mais próximo no alcance.
#[allow(clippy::too_many_arguments)]
pub fn auto_fire(
    mut commands: Commands,
    tuning: Res<CombatTuning>,
    map: Res<ServerWorldMap>,
    risk: Res<ServerRiskPolicy>,
    reputation: Res<Reputation>,
    mut projectile_ids: ResMut<ProjectileIdCounter>,
    mut ships: Query<&mut ServerShip>,
    npcs: Query<&NpcShip>,
    parties: Res<crate::party::Parties>,
) {
    let seen = contacts(&ships, &npcs);
    for mut ship in &mut ships {
        if ship.fire_target.is_some() {
            ship.fire_target = None;
        }
        if cannons_cold(&ship, &map, &risk) {
            continue;
        }
        let weapon = weapon_of(&ship, &tuning);
        let me = (ship.motion.x, ship.motion.y);
        let shooter = (ship.ship_id, ship.character);
        if let Some(lock) = ship.target_lock {
            let kept = seen.iter().any(|c| {
                c.ship_id == lock
                    && (c.at.0 - me.0).hypot(c.at.1 - me.1) <= weapon.range * LOCK_KEEP
            });
            if !kept {
                ship.target_lock = None;
            }
        }
        let raised = ship.black_flag.is_raised();
        let lock = ship.target_lock;
        let target = lock
            .and_then(|id| {
                nearest_in_range(
                    me,
                    weapon.range,
                    seen.iter()
                        .filter(|c| c.ship_id == id)
                        .map(|c| (c.ship_id, c.at)),
                )
            })
            .or_else(|| {
                nearest_in_range(
                    me,
                    weapon.range,
                    seen.iter()
                        .filter(|c| auto_hostile(shooter, raised, c, &reputation, &parties))
                        .map(|c| (c.ship_id, c.at)),
                )
            });
        let Some((target_id, at)) = target else {
            continue;
        };
        ship.fire_target = Some(target_id);
        // v54: o capitão atirou na mão há pouco — o automático espera.
        if ship.combat.auto_silenced() {
            continue;
        }
        let (side, aim) = aim_at(ship.motion.heading, me, at);
        let reload = reload_secs(&ship, &tuning);
        if !ship.battery.try_fire(side, reload) {
            continue;
        }
        ship.black_flag.fired();
        // v54: o automático é o modo passivo — bate menos que a mira.
        let weapon =
            marvyr_domain_combat::active::scaled(weapon, marvyr_domain_combat::active::AUTO_DAMAGE);
        let salvo = salvo(&ship, &mut projectile_ids, &tuning, weapon, side, aim);
        info!(ship_id = ship.ship_id, target_id, ?side, "tiro automático");
        commands.spawn_batch(salvo.into_iter().map(|p| (ServerProjectile(p),)));
    }
}

/// Recarga do bordo: canhão sem gente carrega devagar (MV-061); afixo de
/// Recarga e Frasco de Fúria aceleram.
pub(crate) fn reload_secs(ship: &ServerShip, tuning: &CombatTuning) -> f32 {
    tuning.cooldown_secs
        * ship.stats.reload_factor
        * ship.flasks.reload_multiplier()
        // v59: artilheiro a bordo.
        * ship.sea.officers.reload_multiplier()
        * marvyr_domain_ships::reload_multiplier(
            ship.sea.crew,
            marvyr_domain_ships::crew_capacity(ship.kind),
        )
}

/// Salva do bordo `side`, girada `aim` rad até a marcação (MV-061).
pub(crate) fn salvo(
    ship: &ServerShip,
    projectile_ids: &mut ProjectileIdCounter,
    tuning: &CombatTuning,
    weapon: WeaponParams,
    side: marvyr_domain_combat::BroadsideSide,
    aim: f32,
) -> Vec<Projectile> {
    let projectile_id = projectile_ids.0;
    projectile_ids.0 += tuning.salvo_balls.max(1);
    let mut salvo = Projectile::broadside_salvo(
        projectile_id,
        ship.ship_id,
        side,
        ship.motion.x,
        ship.motion.y,
        ship.motion.heading,
        ship.motion.speed,
        weapon,
        tuning.salvo_balls,
        tuning.salvo_spacing,
        ship.ammo,
    );
    for ball in &mut salvo {
        ball.rotate(aim);
    }
    salvo
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(ship_id: u32, character: CharacterId) -> Contact {
        Contact {
            ship_id,
            at: (0.0, 0.0),
            kind: ContactKind::Player { character },
        }
    }

    fn npc(ship_id: u32, role: NpcRole, hunting: Option<u32>) -> Contact {
        Contact {
            ship_id,
            at: (0.0, 0.0),
            kind: ContactKind::Npc { role, hunting },
        }
    }

    #[test]
    fn innocents_are_never_auto_targets_without_the_black_flag() {
        let reputation = Reputation::default();
        let me = (1, CharacterId::new());
        let honest = player(2, CharacterId::new());
        let caravan = npc(3, NpcRole::Caravan { reverse: false }, None);
        let navy = npc(4, NpcRole::Navy, None);
        for contact in [honest, caravan, navy] {
            assert!(!auto_hostile(
                me,
                false,
                &contact,
                &reputation,
                &crate::party::Parties::default()
            ));
            assert!(
                auto_hostile(
                    me,
                    true,
                    &contact,
                    &reputation,
                    &crate::party::Parties::default()
                ),
                "içada: todos"
            );
        }
        assert!(
            !auto_hostile(
                me,
                true,
                &player(1, me.1),
                &reputation,
                &crate::party::Parties::default()
            ),
            "nunca a si"
        );
    }

    #[test]
    fn party_mates_are_never_auto_targets_even_under_the_black_flag() {
        let mut reputation = Reputation::default();
        let me = (1, CharacterId::new());
        let mate = CharacterId::new();
        reputation.set_black_flag(mate, true);
        let mut parties = crate::party::Parties::default();
        parties.invite(me.1, mate, 0.0).unwrap();
        parties.answer(mate, true, 0.0).unwrap();
        assert!(!auto_hostile(
            me,
            true,
            &player(2, mate),
            &reputation,
            &parties
        ));
        assert!(auto_hostile(
            me,
            true,
            &player(3, CharacterId::new()),
            &reputation,
            &parties
        ));
    }

    #[test]
    fn outlaws_attackers_and_hunters_are_auto_targets() {
        let mut reputation = Reputation::default();
        let me = (1, CharacterId::new());
        assert!(auto_hostile(
            me,
            false,
            &npc(3, NpcRole::Pirate, None),
            &reputation,
            &crate::party::Parties::default()
        ));
        assert!(auto_hostile(
            me,
            false,
            &npc(4, NpcRole::Navy, Some(1)),
            &reputation,
            &crate::party::Parties::default()
        ));
        assert!(!auto_hostile(
            me,
            false,
            &npc(4, NpcRole::Navy, Some(9)),
            &reputation,
            &crate::party::Parties::default()
        ));

        let flagged = CharacterId::new();
        reputation.set_black_flag(flagged, true);
        assert!(auto_hostile(
            me,
            false,
            &player(5, flagged),
            &reputation,
            &crate::party::Parties::default()
        ));

        let attacker = CharacterId::new();
        reputation.register_hit(attacker, me.1);
        assert!(
            auto_hostile(
                me,
                false,
                &player(6, attacker),
                &reputation,
                &crate::party::Parties::default()
            ),
            "revide"
        );
    }
}
