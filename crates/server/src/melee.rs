//! v61: duelo de abordagem. O `handle_board` valida e prende os ganchos;
//! aqui correm as rodadas (`marvyr_domain_combat::melee`): cada lado escolhe
//! a tática, o NPC anuncia a dele (e às vezes blefa), as baixas caem no
//! convés e, no fim, o navio rende ou a abordagem é repelida. Enquanto dura,
//! os dois cascos ficam presos (sem velocidade).

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_combat::melee::{self, Melee, RoundResult, Tactic};
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{ActionKind, BoardTactic, MeleeUpdate, WorldEventKind};
use tracing::info;

use crate::net::{DeferredImpacts, Impact, Metrics, ReliableChannel, ServerShip};
use crate::npc::NpcShip;
use crate::seafaring::{send_action, BoardTarget, NpcBoardings};

#[derive(Debug, Clone)]
pub struct Duel {
    attacker_id: u32,
    attacker_client: ClientId,
    target: BoardTarget,
    melee: Melee,
    clock: f32,
    attack: Option<Tactic>,
    defend: Option<Tactic>,
    /// NPC: o que anunciou e o que vai jogar nesta rodada.
    npc_round: Option<(Tactic, Tactic)>,
    last: Option<(Tactic, Tactic, RoundResult)>,
}

#[derive(Resource, Default, Debug)]
pub struct Melees(Vec<Duel>);

impl Melees {
    pub fn busy(&self, ship_id: u32) -> bool {
        self.0
            .iter()
            .any(|duel| duel.attacker_id == ship_id || duel.target.ship_id == ship_id)
    }

    pub(crate) fn start(
        &mut self,
        attacker_id: u32,
        attacker_client: ClientId,
        attacker_crew: u16,
        target: BoardTarget,
    ) {
        let mut duel = Duel {
            attacker_id,
            attacker_client,
            target,
            melee: Melee::new(attacker_crew, target.crew, target.hull_ratio),
            clock: melee::ROUND_SECS,
            attack: None,
            defend: None,
            npc_round: None,
            last: None,
        };
        duel.next_npc_round();
        self.0.push(duel);
    }
}

impl Duel {
    fn next_npc_round(&mut self) {
        self.npc_round = self
            .target
            .npc
            .then(|| melee::npc_round(crate::seafaring::roll() as u32));
    }

    /// O painel de um dos lados.
    fn view(&self, attacker: bool, active: bool, captured: bool) -> MeleeUpdate {
        let m = &self.melee;
        let (my_crew, their_crew, my_wins, their_wins) = if attacker {
            (
                m.attacker_crew,
                m.defender_crew,
                m.attacker_wins,
                m.defender_wins,
            )
        } else {
            (
                m.defender_crew,
                m.attacker_crew,
                m.defender_wins,
                m.attacker_wins,
            )
        };
        let pick = if attacker { self.attack } else { self.defend };
        let last = self.last.map(|(a, d, result)| {
            let (mine, theirs) = if attacker { (a, d) } else { (d, a) };
            let score = match (result, attacker) {
                (RoundResult::Draw, _) => 0,
                (RoundResult::Attacker, true) | (RoundResult::Defender, false) => 1,
                _ => -1,
            };
            (mine.code(), theirs.code(), score)
        });
        MeleeUpdate {
            active,
            attacker,
            round: m.round.min(melee::ROUNDS),
            secs_left: self.clock.max(0.0),
            my_crew,
            their_crew,
            my_wins,
            their_wins,
            hint: self
                .npc_round
                .filter(|_| attacker)
                .map_or(0, |(intent, _)| intent.code()),
            my_pick: pick.map_or(0, Tactic::code),
            last,
            captured,
        }
    }

    fn send(&self, connection_manager: &mut ConnectionManager, active: bool, captured: bool) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(
            self.attacker_client,
            &self.view(true, active, captured),
        );
        if let Some(defender) = self.target.player {
            let _ = connection_manager
                .send_message::<ReliableChannel, _>(defender, &self.view(false, active, captured));
        }
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<Melees>();
}

#[allow(clippy::too_many_arguments)]
pub fn run_melees(
    time: Res<Time>,
    mut melees: ResMut<Melees>,
    mut tactics: EventReader<ServerReceiveMessage<BoardTactic>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut ships: Query<&mut ServerShip>,
    mut npcs: Query<&mut NpcShip>,
    mut deferred: ResMut<DeferredImpacts>,
    mut npc_boardings: ResMut<NpcBoardings>,
    mut metrics: ResMut<Metrics>,
) {
    for event in tactics.read() {
        let Some(tactic) = Tactic::from_code(event.message().tactic) else {
            continue;
        };
        let from = event.from();
        let Some(duel) = melees
            .0
            .iter_mut()
            .find(|duel| duel.attacker_client == from || duel.target.player == Some(from))
        else {
            continue;
        };
        // Escolha vale uma vez por rodada.
        let attacker = duel.attacker_client == from;
        let slot = if attacker {
            &mut duel.attack
        } else {
            &mut duel.defend
        };
        if slot.is_none() {
            *slot = Some(tactic);
            let _ = connection_manager
                .send_message::<ReliableChannel, _>(from, &duel.view(attacker, true, false));
        }
    }

    let dt = time.delta_secs();
    let mut finished = Vec::new();
    for (index, duel) in melees.0.iter_mut().enumerate() {
        // Ganchos presos: ninguém sai andando. Casco que sumiu (afundou,
        // desconectou) ou atracou encerra sem rendição (abordagem é coisa de
        // mar aberto; o `boarded` não pode cair num navio no porto).
        let mut attacker_here = false;
        let mut target_here = false;
        for mut ship in &mut ships {
            let at_sea = ship.presence == VesselPresence::AtSea;
            if ship.ship_id == duel.attacker_id {
                ship.motion.speed = 0.0;
                attacker_here = at_sea;
            } else if ship.ship_id == duel.target.ship_id {
                ship.motion.speed = 0.0;
                target_here = at_sea;
            }
        }
        for mut npc in &mut npcs {
            if npc.ship_id == duel.target.ship_id {
                npc.motion.speed = 0.0;
                target_here = true;
            }
        }
        if !attacker_here || !target_here {
            duel.send(&mut connection_manager, false, false);
            finished.push((index, None));
            continue;
        }
        duel.clock -= dt;
        let defender_ready = duel.target.npc || duel.defend.is_some();
        if duel.clock > 0.0 && !(duel.attack.is_some() && defender_ready) {
            continue;
        }
        // Sem escolha, o convés se fecha em Muralha.
        let attack = duel.attack.unwrap_or(Tactic::Hold);
        let defend = match duel.npc_round {
            Some((_, played)) => played,
            None => duel.defend.unwrap_or(Tactic::Hold),
        };
        let (result, a_loss, d_loss) = duel.melee.resolve(attack, defend);
        for mut ship in &mut ships {
            if ship.ship_id == duel.attacker_id {
                ship.sea.crew -= a_loss.min(ship.sea.crew);
            } else if ship.ship_id == duel.target.ship_id {
                ship.sea.crew -= d_loss.min(ship.sea.crew);
            }
        }
        duel.last = Some((attack, defend, result));
        duel.attack = None;
        duel.defend = None;
        duel.clock = melee::ROUND_SECS;
        duel.next_npc_round();
        match duel.melee.outcome() {
            Some(captured) => {
                duel.send(&mut connection_manager, false, captured);
                finished.push((index, Some(captured)));
            }
            None => duel.send(&mut connection_manager, true, false),
        }
    }

    for (index, captured) in finished.into_iter().rev() {
        let duel = melees.0.remove(index);
        if let Some(captured) = captured {
            finish(
                &duel,
                captured,
                &mut connection_manager,
                &mut ships,
                &mut deferred,
                &mut npc_boardings,
                &mut metrics,
            );
        }
    }
}

/// Fim do duelo: rendição (o navio vai para o vencedor como antes do v61)
/// ou abordagem repelida.
fn finish(
    duel: &Duel,
    captured: bool,
    connection_manager: &mut ConnectionManager,
    ships: &mut Query<&mut ServerShip>,
    deferred: &mut DeferredImpacts,
    npc_boardings: &mut NpcBoardings,
    metrics: &mut Metrics,
) {
    let target = duel.target;
    let attacker_losses = duel.melee.attacker_losses;
    let defender_losses = duel.melee.defender_losses;
    info!(
        attacker = duel.attacker_id,
        target = target.ship_id,
        captured,
        attacker_losses,
        defender_losses,
        "abordagem"
    );
    if !captured {
        send_action(
            connection_manager,
            duel.attacker_client,
            ActionKind::Board,
            false,
            format!("Abordagem repelida! {attacker_losses} marujos perdidos."),
        );
        if let Some(victim) = target.player {
            crate::reputation::send_event(
                connection_manager,
                &[victim],
                format!("Abordagem repelida! {defender_losses} marujos perdidos."),
                WorldEventKind::Alert,
            );
        }
        return;
    }
    metrics.boardings_won += 1;
    // v59: navio de jogador rendido entrega um oficial que falta ao
    // atacante (sai do rendido: o oficial mora num lugar só).
    let prize = ships
        .iter()
        .find(|s| s.ship_id == duel.attacker_id)
        .and_then(|s| s.sea.officers.capturable_from(target.officers));
    if let Some(officer) = prize {
        for mut ship in ships.iter_mut() {
            if ship.ship_id == duel.attacker_id {
                ship.sea.officers.add(officer);
            } else if ship.ship_id == target.ship_id {
                ship.sea.officers.remove(officer);
            }
        }
    }
    if target.npc {
        npc_boardings.0.push((target.ship_id, duel.attacker_id));
    } else {
        deferred.0.push(Impact {
            projectile: None,
            target_ship_id: target.ship_id,
            hull_damage: 0,
            attacker_ship_id: duel.attacker_id,
            sail_damage: 0.0,
            at: (target.x, target.y),
            boarded: true,
        });
    }
    send_action(
        connection_manager,
        duel.attacker_client,
        ActionKind::Board,
        true,
        match prize {
            Some(officer) => format!(
                "Abordagem vencida! O navio rendeu ({attacker_losses} baixas) e o {} dele agora é seu.",
                officer.name()
            ),
            None => format!("Abordagem vencida! O navio rendeu ({attacker_losses} baixas)."),
        },
    );
    if let Some(victim) = target.player {
        crate::reputation::send_event(
            connection_manager,
            &[victim],
            String::from("Seu navio foi tomado por abordagem!"),
            WorldEventKind::Kill,
        );
    }
}
