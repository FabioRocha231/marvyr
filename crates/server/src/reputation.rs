//! Reputação / notoriedade (MF-059): atacar quem é honesto fora das águas
//! protegidas deixa marca. A marca decai sozinha; acumulada, vira cabeça a
//! prêmio — a marinha caça, os portos da coroa fecham e quem afunda o
//! procurado limpa a ficha dele sem sujar a própria. Regras puras em cima,
//! sistemas embaixo.
//!
//! ponytail: notoriedade vive só em memória — restart do servidor perdoa
//! todo mundo. Persistir junto do personagem quando a temporada importar.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::RiskTier;
use marvyr_protocol::{
    ReputationUpdate, WorldEvent, WorldEventKind, TIER_HONRADO, TIER_PROCURADO, TIER_SUSPEITO,
};
use marvyr_shared::ids::CharacterId;

use crate::net::{
    CombatImpacts, PendingShipDestructions, ReliableChannel, ServerShip, ServerWorldMap,
};

pub const MAX_NOTORIETY: u32 = 1000;
/// Por impacto em caravana ou jogador honesto (águas Frontier).
pub const HIT_GAIN: u32 = 15;
/// Por afundamento de caravana ou jogador honesto (águas Frontier).
pub const SINK_GAIN: u32 = 120;
/// -1 de notoriedade a cada intervalo.
pub const DECAY_EVERY_SECS: f32 = 10.0;
pub const SUSPEITO_AT: u32 = 100;
pub const PROCURADO_AT: u32 = 300;
/// Quem ataca uma caravana fica marcado pela marinha por este tempo.
pub const CROWN_AGGRESSOR_SECS: f32 = 60.0;
/// Quanto tempo um ataque dá direito a revide sem sujar a ficha.
pub const AGGRESSION_SECS: f32 = 60.0;
/// Balas do mesmo par dentro desta janela contam como um acerto só.
const HIT_WINDOW_SECS: f32 = 1.0;
/// Portos da coroa: recusam Procurados. Um porto pirata fica de fora da
/// lista e atraca qualquer um.
pub const CROWN_PORTS: [&str; 2] = ["Porto da Serra", "Porto da Mina"];
pub const CROWN_REFUSAL: &str = "Procurado: os portos da coroa recusam seu navio";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Honrado,
    Suspeito,
    Procurado,
}

impl Tier {
    pub fn of(notoriety: u32) -> Self {
        if notoriety >= PROCURADO_AT {
            Self::Procurado
        } else if notoriety >= SUSPEITO_AT {
            Self::Suspeito
        } else {
            Self::Honrado
        }
    }

    pub fn wire(self) -> u8 {
        match self {
            Self::Honrado => TIER_HONRADO,
            Self::Suspeito => TIER_SUSPEITO,
            Self::Procurado => TIER_PROCURADO,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offense {
    Hit,
    Sink,
}

/// Notoriedade ganha pelo agressor. `victim_is_outlaw` = pirata ou
/// Procurado: caçar fora-da-lei nunca suja o nome. A zona é a da VÍTIMA;
/// Lawless conta metade, Protected/fora do mapa nada (o dano nem entra).
pub fn notoriety_gain(
    offense: Offense,
    victim_zone: Option<RiskTier>,
    victim_is_outlaw: bool,
) -> u32 {
    if victim_is_outlaw {
        return 0;
    }
    let base = match offense {
        Offense::Hit => HIT_GAIN,
        Offense::Sink => SINK_GAIN,
    };
    match victim_zone {
        Some(RiskTier::Frontier) => base,
        Some(RiskTier::Lawless) => base / 2,
        _ => 0,
    }
}

/// Porto da coroa recusa Procurado; qualquer outro porto atraca.
pub fn dock_refusal(port: &str, notoriety: u32) -> Option<&'static str> {
    (Tier::of(notoriety) == Tier::Procurado && CROWN_PORTS.contains(&port)).then_some(CROWN_REFUSAL)
}

#[derive(Resource, Debug, Default)]
pub struct Reputation {
    notoriety: HashMap<CharacterId, u32>,
    /// Segundos restantes de marca de "atacou caravana" (marinha caça).
    crown_aggressors: HashMap<CharacterId, f32>,
    /// (atacante, vítima) → segundos desde que a marca foi posta, contando
    /// para baixo. Revidar quem te atacou não suja a ficha, e cada par conta
    /// no máximo um acerto por segundo (uma salva = um acerto).
    aggression: HashMap<(CharacterId, CharacterId), f32>,
    /// Capitães com a Bandeira Negra içada (espelho do `ServerShip`).
    black_flags: HashSet<CharacterId>,
    decay_clock: f32,
}

impl Reputation {
    pub fn notoriety(&self, character: CharacterId) -> u32 {
        self.notoriety.get(&character).copied().unwrap_or(0)
    }

    pub fn tier(&self, character: CharacterId) -> Tier {
        Tier::of(self.notoriety(character))
    }

    /// Soma (com teto) e devolve a faixa ANTERIOR para anúncio de subida.
    pub fn add(&mut self, character: CharacterId, amount: u32) -> Tier {
        let before = self.tier(character);
        if amount > 0 {
            let value = self.notoriety.entry(character).or_default();
            *value = (*value + amount).min(MAX_NOTORIETY);
        }
        before
    }

    /// Morte zera a ficha. Devolve o valor anterior.
    pub fn reset(&mut self, character: CharacterId) -> u32 {
        self.crown_aggressors.remove(&character);
        self.notoriety.remove(&character).unwrap_or(0)
    }

    /// Registra o acerto de `attacker` em `victim` e diz se ele conta para a
    /// notoriedade: não conta se for revide nem se o par acertou há < 1 s.
    pub fn register_hit(&mut self, attacker: CharacterId, victim: CharacterId) -> bool {
        // Revide não vira agressão: senão quem começou a briga passava a
        // contar como "revidando" e afundava o outro de graça.
        if self.aggression.contains_key(&(victim, attacker)) {
            return false;
        }
        let same_salvo = self
            .aggression
            .get(&(attacker, victim))
            .is_some_and(|secs| *secs > AGGRESSION_SECS - HIT_WINDOW_SECS);
        self.aggression.insert((attacker, victim), AGGRESSION_SECS);
        !same_salvo
    }

    /// Afundar quem te atacou há pouco é legítima defesa.
    pub fn is_self_defense(&self, killer: CharacterId, victim: CharacterId) -> bool {
        self.aggression.contains_key(&(victim, killer))
    }

    pub fn mark_crown_aggressor(&mut self, character: CharacterId) {
        self.crown_aggressors
            .insert(character, CROWN_AGGRESSOR_SECS);
    }

    /// A marinha caça fora-da-lei e quem atacou caravana há pouco.
    pub fn hunted_by_navy(&self, character: CharacterId) -> bool {
        self.is_outlaw(character) || self.crown_aggressors.contains_key(&character)
    }

    pub fn set_black_flag(&mut self, character: CharacterId, raised: bool) {
        if raised {
            self.black_flags.insert(character);
        } else {
            self.black_flags.remove(&character);
        }
    }

    pub fn black_flag(&self, character: CharacterId) -> bool {
        self.black_flags.contains(&character)
    }

    /// Procurado ou de Bandeira Negra: caçá-lo nunca suja a ficha, e o tiro
    /// automático de todo mundo mira nele.
    pub fn is_outlaw(&self, character: CharacterId) -> bool {
        self.tier(character) == Tier::Procurado || self.black_flag(character)
    }

    /// `victim` acertou `me` há pouco (revide automático).
    pub fn attacked_me(&self, me: CharacterId, victim: CharacterId) -> bool {
        self.aggression.contains_key(&(victim, me))
    }

    /// Avança decaimento e marcas; devolve quem teve a notoriedade mudada.
    /// `at_sea` = quem decai: deslogado ou atracado não limpa a ficha
    /// esperando.
    pub fn tick(&mut self, dt: f32, at_sea: impl Fn(CharacterId) -> bool) -> Vec<CharacterId> {
        self.crown_aggressors.retain(|_, secs| {
            *secs -= dt;
            *secs > 0.0
        });
        self.aggression.retain(|_, secs| {
            *secs -= dt;
            *secs > 0.0
        });
        self.decay_clock += dt;
        if self.decay_clock < DECAY_EVERY_SECS {
            return Vec::new();
        }
        self.decay_clock -= DECAY_EVERY_SECS;
        let changed: Vec<CharacterId> = self
            .notoriety
            .keys()
            .copied()
            .filter(|character| at_sea(*character))
            .collect();
        for character in &changed {
            if let Some(value) = self.notoriety.get_mut(character) {
                *value -= 1;
            }
        }
        self.notoriety.retain(|_, value| *value > 0);
        changed
    }

    pub fn update_for(&self, character: CharacterId) -> ReputationUpdate {
        let notoriety = self.notoriety(character);
        ReputationUpdate {
            notoriety,
            tier: Tier::of(notoriety).wire(),
        }
    }
}

pub(crate) fn send_reputation(
    connection_manager: &mut ConnectionManager,
    reputation: &Reputation,
    client_id: Option<ClientId>,
    character: CharacterId,
) {
    if let Some(client_id) = client_id {
        let _ = connection_manager
            .send_message::<ReliableChannel, _>(client_id, &reputation.update_for(character));
    }
}

pub(crate) fn send_event(
    connection_manager: &mut ConnectionManager,
    clients: &[ClientId],
    text: String,
    kind: WorldEventKind,
) {
    if clients.is_empty() {
        return;
    }
    let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
        &WorldEvent { text, kind },
        NetworkTarget::Only(clients.to_vec()),
    );
}

/// Subiu de faixa: o próprio capitão fica sabendo pelo feed.
pub(crate) fn announce_tier_rise(
    connection_manager: &mut ConnectionManager,
    reputation: &Reputation,
    client_id: Option<ClientId>,
    character: CharacterId,
    before: Tier,
) {
    let (Some(client_id), after) = (client_id, reputation.tier(character)) else {
        return;
    };
    if after <= before {
        return;
    }
    let (text, kind) = match after {
        Tier::Procurado => (
            String::from("PROCURADO: a marinha caca voce"),
            WorldEventKind::Bounty,
        ),
        _ => (
            String::from("A coroa agora o considera SUSPEITO"),
            WorldEventKind::Alert,
        ),
    };
    send_event(connection_manager, &[client_id], text, kind);
}

/// Soma notoriedade e avisa o dono (valor novo + subida de faixa).
pub(crate) fn raise_notoriety(
    connection_manager: &mut ConnectionManager,
    reputation: &mut Reputation,
    client_id: Option<ClientId>,
    character: CharacterId,
    gain: u32,
) {
    if gain == 0 {
        return;
    }
    let before = reputation.add(character, gain);
    send_reputation(connection_manager, reputation, client_id, character);
    announce_tier_rise(connection_manager, reputation, client_id, character, before);
}

fn zone_tier(map: &ServerWorldMap, x: f32, y: f32) -> Option<RiskTier> {
    map.0.zone_at(x, y).ok().map(|zone| zone.tier)
}

/// Impactos jogador→jogador ainda não aplicados (corre antes do dano, com a
/// ficha da vítima de antes do naufrágio).
pub fn track_player_hits(
    mut connection_manager: ResMut<ConnectionManager>,
    impacts: Res<CombatImpacts>,
    map: Res<ServerWorldMap>,
    ships: Query<&ServerShip>,
    mut reputation: ResMut<Reputation>,
) {
    for impact in &impacts.0 {
        let find = |id: u32| ships.iter().find(|ship| ship.ship_id == id);
        let (Some(attacker), Some(victim)) =
            (find(impact.attacker_ship_id), find(impact.target_ship_id))
        else {
            continue;
        };
        if attacker.character == victim.character
            || !reputation.register_hit(attacker.character, victim.character)
        {
            continue;
        }
        let gain = notoriety_gain(
            Offense::Hit,
            zone_tier(&map, victim.motion.x, victim.motion.y),
            reputation.is_outlaw(victim.character),
        );
        raise_notoriety(
            &mut connection_manager,
            &mut reputation,
            attacker.client_id,
            attacker.character,
            gain,
        );
    }
}

/// Naufrágios de jogador: afundar Procurado não suja a ficha, afundar
/// honesto soma notoriedade, e a ficha de quem morreu zera.
pub fn settle_player_sinks(
    mut connection_manager: ResMut<ConnectionManager>,
    pending: Res<PendingShipDestructions>,
    map: Res<ServerWorldMap>,
    ships: Query<&ServerShip>,
    mut reputation: ResMut<Reputation>,
) {
    let viewers: Vec<(Option<ClientId>, CharacterId)> = ships
        .iter()
        .map(|ship| (ship.client_id, ship.character))
        .collect();
    for destruction in &pending.0 {
        let victim = destruction.victim_character;
        let victim_notoriety = reputation.notoriety(victim);
        let killer = destruction
            .exclusive_looter
            .filter(|killer| *killer != victim);
        if let Some(killer) = killer {
            let killer_client = viewers
                .iter()
                .find(|(_, owner)| *owner == killer)
                .and_then(|(client, _)| *client);
            if Tier::of(victim_notoriety) == Tier::Procurado {
                if let Some(client) = killer_client {
                    send_event(
                        &mut connection_manager,
                        &[client],
                        String::from("Voce afundou um Procurado"),
                        WorldEventKind::Kill,
                    );
                }
            } else {
                let gain = if reputation.is_self_defense(killer, victim)
                    || reputation.black_flag(victim)
                {
                    0
                } else {
                    notoriety_gain(
                        Offense::Sink,
                        zone_tier(&map, destruction.victim_x, destruction.victim_y),
                        false,
                    )
                };
                if let Some(client) = killer_client {
                    send_event(
                        &mut connection_manager,
                        &[client],
                        String::from("Voce afundou um navio"),
                        WorldEventKind::Kill,
                    );
                }
                raise_notoriety(
                    &mut connection_manager,
                    &mut reputation,
                    killer_client,
                    killer,
                    gain,
                );
            }
        }
        // Só outro capitão limpa a ficha: afundar para um NPC (ou de
        // propósito) não apaga a cabeça a prêmio.
        if killer.is_some() && reputation.reset(victim) > 0 {
            send_reputation(
                &mut connection_manager,
                &reputation,
                destruction.victim_client_id,
                victim,
            );
            if Tier::of(victim_notoriety) == Tier::Procurado {
                if let Some(client) = destruction.victim_client_id {
                    send_event(
                        &mut connection_manager,
                        &[client],
                        String::from("Sua cabeca foi cobrada. Ficha limpa."),
                        WorldEventKind::Bounty,
                    );
                }
            }
        }
    }
}

pub fn decay_notoriety(
    time: Res<Time>,
    mut connection_manager: ResMut<ConnectionManager>,
    ships: Query<&ServerShip>,
    mut reputation: ResMut<Reputation>,
) {
    let at_sea = |character: CharacterId| {
        ships.iter().any(|ship| {
            ship.character == character
                && ship.client_id.is_some()
                && ship.presence == VesselPresence::AtSea
        })
    };
    for character in reputation.tick(time.delta_secs(), at_sea) {
        if let Some(ship) = ships.iter().find(|ship| ship.character == character) {
            send_reputation(
                &mut connection_manager,
                &reputation,
                ship.client_id,
                character,
            );
        }
    }
}

/// Navio novo (hello, respawn, reconnect): o dono recebe a ficha atual.
/// Dev tooling (PRD §39): `MARVYR_DEV_NOTORIETY=N` faz todo capitão
/// novo nascer com N de notoriedade — revisar selo, marca e marinha sem
/// afundar caravana. Não é mecânica de jogo.
pub fn announce_reputation_on_spawn(
    mut connection_manager: ResMut<ConnectionManager>,
    ships: Query<&ServerShip, Added<ServerShip>>,
    mut reputation: ResMut<Reputation>,
    mut dev_notoriety: Local<Option<Option<u32>>>,
) {
    // Lido uma vez (o sistema roda a 60 Hz).
    let dev_notoriety = *dev_notoriety.get_or_insert_with(|| {
        std::env::var("MARVYR_DEV_NOTORIETY")
            .ok()
            .and_then(|value| value.parse().ok())
    });
    for ship in &ships {
        if let Some(value) = dev_notoriety.filter(|_| reputation.notoriety(ship.character) == 0) {
            reputation.add(ship.character, value);
        }
        send_reputation(
            &mut connection_manager,
            &reputation,
            ship.client_id,
            ship.character,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_split_at_100_and_300() {
        assert_eq!(Tier::of(0), Tier::Honrado);
        assert_eq!(Tier::of(99), Tier::Honrado);
        assert_eq!(Tier::of(100), Tier::Suspeito);
        assert_eq!(Tier::of(299), Tier::Suspeito);
        assert_eq!(Tier::of(300), Tier::Procurado);
    }

    #[test]
    fn gains_are_full_in_frontier_half_in_lawless_and_never_on_outlaws() {
        use Offense::*;
        assert_eq!(notoriety_gain(Hit, Some(RiskTier::Frontier), false), 15);
        assert_eq!(notoriety_gain(Sink, Some(RiskTier::Frontier), false), 120);
        assert_eq!(notoriety_gain(Hit, Some(RiskTier::Lawless), false), 7);
        assert_eq!(notoriety_gain(Sink, Some(RiskTier::Lawless), false), 60);
        assert_eq!(notoriety_gain(Hit, Some(RiskTier::Protected), false), 0);
        assert_eq!(notoriety_gain(Hit, None, false), 0);
        assert_eq!(notoriety_gain(Sink, Some(RiskTier::Frontier), true), 0);
    }

    #[test]
    fn notoriety_caps_decays_and_resets() {
        let mut rep = Reputation::default();
        let captain = CharacterId::new();
        assert_eq!(rep.add(captain, 250), Tier::Honrado);
        assert_eq!(rep.add(captain, 120), Tier::Suspeito);
        assert_eq!(rep.tier(captain), Tier::Procurado);
        rep.add(captain, 5_000);
        assert_eq!(rep.notoriety(captain), MAX_NOTORIETY);

        assert!(rep.tick(DECAY_EVERY_SECS - 0.5, |_| true).is_empty());
        assert_eq!(rep.tick(1.0, |_| true), vec![captain]);
        assert_eq!(rep.notoriety(captain), MAX_NOTORIETY - 1);

        assert_eq!(rep.reset(captain), MAX_NOTORIETY - 1);
        assert_eq!(rep.notoriety(captain), 0);
        assert_eq!(rep.update_for(captain).notoriety, 0);
    }

    #[test]
    fn decay_forgets_captains_at_zero() {
        let mut rep = Reputation::default();
        let captain = CharacterId::new();
        rep.add(captain, 1);
        rep.tick(DECAY_EVERY_SECS, |_| true);
        assert_eq!(rep.notoriety(captain), 0);
        assert!(rep.tick(DECAY_EVERY_SECS, |_| true).is_empty());
    }

    #[test]
    fn crown_ports_refuse_wanted_captains_only() {
        assert_eq!(dock_refusal("Porto da Serra", 300), Some(CROWN_REFUSAL));
        assert_eq!(dock_refusal("Porto da Mina", 999), Some(CROWN_REFUSAL));
        assert_eq!(dock_refusal("Porto da Serra", 299), None);
        // Porto fora da coroa (ex.: porto pirata) atraca qualquer um.
        assert_eq!(dock_refusal("Porto do Coral Negro", 999), None);
    }

    #[test]
    fn navy_hunts_wanted_and_recent_caravan_aggressors() {
        let mut rep = Reputation::default();
        let honest = CharacterId::new();
        let wanted = CharacterId::new();
        let raider = CharacterId::new();
        rep.add(wanted, PROCURADO_AT);
        rep.mark_crown_aggressor(raider);
        assert!(!rep.hunted_by_navy(honest));
        assert!(rep.hunted_by_navy(wanted));
        assert!(rep.hunted_by_navy(raider));
        rep.tick(CROWN_AGGRESSOR_SECS + 1.0, |_| true);
        assert!(!rep.hunted_by_navy(raider));
    }

    #[test]
    fn retaliation_and_same_salvo_hits_do_not_count() {
        let mut rep = Reputation::default();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        assert!(rep.register_hit(a, b));
        // Segunda bala da mesma salva.
        assert!(!rep.register_hit(a, b));
        // B revida: legítima defesa.
        assert!(!rep.register_hit(b, a));
        assert!(rep.is_self_defense(b, a));
        // Quem começou continua sujando a ficha na salva seguinte.
        rep.tick(HIT_WINDOW_SECS + 0.5, |_| true);
        assert!(rep.register_hit(a, b));
        assert!(!rep.is_self_defense(a, b));
        rep.tick(AGGRESSION_SECS + 1.0, |_| true);
        assert!(!rep.is_self_defense(b, a));
        assert!(rep.register_hit(b, a));
    }
}
