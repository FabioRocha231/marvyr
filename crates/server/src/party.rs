//! Party (v55): até 5 capitães navegando juntos. Na party: bala e choque
//! não machucam companheiro, o tiro automático não mira nele, o saque
//! reservado ao vencedor vale para a party e afundar NPC paga Renome a quem
//! estava por perto. Só da sessão — não persiste (reconectar na janela de
//! graça mantém; sumir do mar tira).

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_protocol::{
    PartyAnswer, PartyInvite, PartyLeave, PartyMember, PartyUpdate, WorldEventKind,
};
use marvyr_shared::ids::CharacterId;

use crate::captains::CaptainNames;
use crate::net::{ReliableChannel, ServerShip};

pub const MAX_PARTY: usize = 5;
/// Convite vale por este tempo.
const INVITE_SECS: f32 = 60.0;
/// A party vai para os membros a cada segundo (casco dos companheiros).
const UPDATE_EVERY_SECS: f32 = 1.0;
/// Renome de NPC afundado vale para quem da party estava a esta distância.
pub const SHARE_RADIUS: f32 = 900.0;

#[derive(Resource, Default, Debug)]
pub struct Parties {
    next_id: u32,
    of: HashMap<CharacterId, u32>,
    members: HashMap<u32, Vec<CharacterId>>,
    /// Convidado → (quem convidou, expira em).
    invites: HashMap<CharacterId, (CharacterId, f32)>,
    /// v62: companhia de cada capitão (companheiro de companhia também é
    /// aliado: sem fogo amigo, sem abordagem, saque reservado junto).
    companies: HashMap<CharacterId, uuid::Uuid>,
}

impl Parties {
    /// Aliados: a mesma party ou a mesma companhia (v62).
    pub fn same_party(&self, a: CharacterId, b: CharacterId) -> bool {
        a == b
            || matches!((self.of.get(&a), self.of.get(&b)), (Some(x), Some(y)) if x == y)
            || matches!(
                (self.companies.get(&a), self.companies.get(&b)),
                (Some(x), Some(y)) if x == y
            )
    }

    /// v62: o `companies` avisa quem é de qual companhia.
    pub fn set_companies(&mut self, companies: HashMap<CharacterId, uuid::Uuid>) {
        self.companies = companies;
    }

    /// Todos da party de `who`, inclusive ele (sozinho: só ele).
    pub fn members_of(&self, who: CharacterId) -> Vec<CharacterId> {
        self.of
            .get(&who)
            .and_then(|id| self.members.get(id))
            .cloned()
            .unwrap_or_else(|| vec![who])
    }

    pub fn invite(
        &mut self,
        from: CharacterId,
        to: CharacterId,
        now: f32,
    ) -> Result<(), &'static str> {
        if from == to {
            return Err("Você já navega consigo mesmo");
        }
        if self.of.contains_key(&to) {
            return Err("Esse capitão já está numa party");
        }
        if self.members_of(from).len() >= MAX_PARTY {
            return Err("Party cheia (5 capitães)");
        }
        self.invites.insert(to, (from, now + INVITE_SECS));
        Ok(())
    }

    /// Quem convidou `who`, se o convite ainda vale.
    pub fn pending_invite(&self, who: CharacterId, now: f32) -> Option<CharacterId> {
        self.invites
            .get(&who)
            .filter(|(_, expires)| *expires > now)
            .map(|(from, _)| *from)
    }

    pub fn answer(
        &mut self,
        who: CharacterId,
        accept: bool,
        now: f32,
    ) -> Result<CharacterId, &'static str> {
        let from = self
            .pending_invite(who, now)
            .ok_or("Nenhum convite de party pendente")?;
        self.invites.remove(&who);
        if !accept {
            return Ok(from);
        }
        if self.of.contains_key(&who) {
            return Err("Você já está numa party");
        }
        if self.members_of(from).len() >= MAX_PARTY {
            return Err("Party cheia (5 capitães)");
        }
        let id = match self.of.get(&from) {
            Some(id) => *id,
            None => {
                self.next_id += 1;
                let id = self.next_id;
                self.of.insert(from, id);
                self.members.insert(id, vec![from]);
                id
            }
        };
        self.of.insert(who, id);
        self.members.entry(id).or_default().push(who);
        Ok(from)
    }

    /// Sai da party; party de um só se desfaz.
    pub fn leave(&mut self, who: CharacterId) {
        let Some(id) = self.of.remove(&who) else {
            return;
        };
        if let Some(members) = self.members.get_mut(&id) {
            members.retain(|member| *member != who);
            if members.len() <= 1 {
                for member in members.drain(..) {
                    self.of.remove(&member);
                }
                self.members.remove(&id);
            }
        }
    }

    fn prune(&mut self, present: &[CharacterId], now: f32) {
        let gone: Vec<CharacterId> = self
            .of
            .keys()
            .filter(|who| !present.contains(who))
            .copied()
            .collect();
        for who in gone {
            self.leave(who);
        }
        self.invites.retain(|to, (from, expires)| {
            *expires > now && present.contains(to) && present.contains(from)
        });
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<Parties>();
}

fn notify(
    connection_manager: &mut ConnectionManager,
    client: Option<ClientId>,
    text: impl Into<String>,
) {
    if let Some(client) = client {
        crate::reputation::send_event(
            connection_manager,
            &[client],
            text.into(),
            WorldEventKind::Alert,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_party_intents(
    time: Res<Time>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut invites: EventReader<ServerReceiveMessage<PartyInvite>>,
    mut answers: EventReader<ServerReceiveMessage<PartyAnswer>>,
    mut leaves: EventReader<ServerReceiveMessage<PartyLeave>>,
    names: Res<CaptainNames>,
    mut parties: ResMut<Parties>,
    ships: Query<&ServerShip>,
) {
    let now = time.elapsed_secs();
    let by_client = |client: ClientId| ships.iter().find(|ship| ship.client_id == Some(client));
    let client_of = |who: CharacterId| {
        ships
            .iter()
            .find(|ship| ship.character == who)
            .and_then(|ship| ship.client_id)
    };
    let name_of = |who: CharacterId| {
        ships
            .iter()
            .find(|ship| ship.character == who)
            .map_or_else(|| String::from("Capitão"), |ship| names.of(ship))
    };
    for event in invites.read() {
        let Some(me) = by_client(event.from()) else {
            continue;
        };
        let Some(target) = ships.iter().find(|ship| {
            ship.ship_id == event.message().target_ship_id && ship.client_id.is_some()
        }) else {
            notify(
                &mut connection_manager,
                me.client_id,
                "Só dá para convidar capitão no mar",
            );
            continue;
        };
        match parties.invite(me.character, target.character, now) {
            Ok(()) => {
                notify(
                    &mut connection_manager,
                    me.client_id,
                    format!("Convite enviado a {}", names.of(target)),
                );
                notify(
                    &mut connection_manager,
                    target.client_id,
                    format!(
                        "{} te chamou para a party: V aceita, Shift+V recusa",
                        names.of(me)
                    ),
                );
            }
            Err(reason) => notify(&mut connection_manager, me.client_id, reason),
        }
    }
    for event in answers.read() {
        let Some(me) = by_client(event.from()) else {
            continue;
        };
        let accept = event.message().accept;
        match parties.answer(me.character, accept, now) {
            Ok(from) => {
                let text = if accept {
                    format!("{} entrou na party", names.of(me))
                } else {
                    format!("{} recusou o convite", names.of(me))
                };
                for member in parties.members_of(from) {
                    notify(&mut connection_manager, client_of(member), text.clone());
                }
                if !accept {
                    notify(&mut connection_manager, me.client_id, "Convite recusado");
                }
            }
            Err(reason) => notify(&mut connection_manager, me.client_id, reason),
        }
    }
    for event in leaves.read() {
        let Some(me) = by_client(event.from()) else {
            continue;
        };
        let before = parties.members_of(me.character);
        if before.len() <= 1 {
            notify(
                &mut connection_manager,
                me.client_id,
                "Você não está numa party",
            );
            continue;
        }
        parties.leave(me.character);
        for member in before {
            notify(
                &mut connection_manager,
                client_of(member),
                format!("{} saiu da party", name_of(me.character)),
            );
        }
    }
}

/// Manda a party (e o convite pendente) a cada membro, e larga quem
/// sumiu do mar.
pub fn broadcast_parties(
    time: Res<Time>,
    mut clock: Local<f32>,
    names: Res<CaptainNames>,
    mut parties: ResMut<Parties>,
    ships: Query<&ServerShip>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    *clock += time.delta_secs();
    if *clock < UPDATE_EVERY_SECS {
        return;
    }
    *clock = 0.0;
    let now = time.elapsed_secs();
    let present: Vec<CharacterId> = ships.iter().map(|ship| ship.character).collect();
    parties.prune(&present, now);
    for ship in &ships {
        let Some(client) = ship.client_id else {
            continue;
        };
        let members = parties
            .members_of(ship.character)
            .into_iter()
            .filter(|member| *member != ship.character)
            .filter_map(|member| ships.iter().find(|other| other.character == member))
            .map(|other| PartyMember {
                ship_id: other.ship_id,
                name: names.of(other),
                hp: other.hp,
                max_hp: other.stats.max_hp,
            })
            .collect();
        let invite_from = parties
            .pending_invite(ship.character, now)
            .and_then(|from| ships.iter().find(|other| other.character == from))
            .map(|other| names.of(other));
        let _ = connection_manager.send_message::<ReliableChannel, _>(
            client,
            &PartyUpdate {
                members,
                invite_from,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captain() -> CharacterId {
        CharacterId::new()
    }

    #[test]
    fn invite_accept_and_leave() {
        let mut parties = Parties::default();
        let (a, b, c) = (captain(), captain(), captain());
        assert!(!parties.same_party(a, b));
        parties.invite(a, b, 0.0).unwrap();
        assert_eq!(parties.pending_invite(b, 1.0), Some(a));
        assert_eq!(parties.answer(b, true, 1.0), Ok(a));
        assert!(parties.same_party(a, b));
        assert_eq!(parties.members_of(a).len(), 2);
        // C recusa: nada muda.
        parties.invite(b, c, 2.0).unwrap();
        assert_eq!(parties.answer(c, false, 3.0), Ok(b));
        assert!(!parties.same_party(a, c));
        // B sai: party de um só se desfaz.
        parties.leave(b);
        assert!(!parties.same_party(a, b));
        assert_eq!(parties.members_of(a), vec![a]);
    }

    #[test]
    fn invites_expire_and_parties_cap_at_five() {
        let mut parties = Parties::default();
        let lead = captain();
        parties.invite(lead, captain(), 0.0).unwrap();
        let late = parties.invites.keys().next().copied().unwrap();
        assert!(
            parties.answer(late, true, INVITE_SECS + 1.0).is_err(),
            "venceu"
        );
        for _ in 0..MAX_PARTY - 1 {
            let member = captain();
            parties.invite(lead, member, 0.0).unwrap();
            parties.answer(member, true, 0.0).unwrap();
        }
        assert_eq!(parties.members_of(lead).len(), MAX_PARTY);
        assert!(parties.invite(lead, captain(), 0.0).is_err());
        assert!(parties.invite(lead, lead, 0.0).is_err());
    }

    #[test]
    fn captains_gone_from_the_sea_leave_the_party() {
        let mut parties = Parties::default();
        let (a, b) = (captain(), captain());
        parties.invite(a, b, 0.0).unwrap();
        parties.answer(b, true, 0.0).unwrap();
        parties.prune(&[a], 1.0);
        assert!(!parties.same_party(a, b));
    }
}
