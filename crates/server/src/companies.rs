//! v62: companhias — guildas de jogador. Funda-se atracado, pagando com o
//! armazém do porto (escambo: sem moeda); convida-se quem está atracado no
//! mesmo porto. Companheiro de companhia é aliado como o de party (sem fogo
//! amigo, sem abordagem), a tag vai na placa e a influência dos membros soma
//! na guerra de território (`territory`). Persiste no Postgres; leitura que
//! falhou não vira "ninguém tem companhia" (MV-067): recusa mudanças até ler.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{
    CompanyAnswer, CompanyInvite, CompanyMember, CompanyUpdate, CreateCompany, LeaveCompany,
    WorldEventKind,
};
use marvyr_shared::ids::CharacterId;
use tracing::{info, warn};
use uuid::Uuid;

use crate::captains::CaptainNames;
use crate::net::{DevItems, ReliableChannel, ServerShip};
use crate::persist::{CompanyRecord, StoreHandle};

pub const MAX_MEMBERS: usize = 30;
/// Fundação: madeira e minério do armazém do porto.
pub const FOUND_TIMBER: u32 = 40;
pub const FOUND_ORE: u32 = 40;
const INVITE_SECS: f32 = 120.0;
const UPDATE_EVERY_SECS: f32 = 2.0;
/// Nova tentativa de ler as companhias depois de falhar.
const RETRY_LOAD_SECS: f32 = 30.0;

pub type Company = CompanyRecord;

#[derive(Resource, Default, Debug)]
pub struct Companies {
    list: Vec<Company>,
    of: HashMap<CharacterId, Uuid>,
    invites: HashMap<CharacterId, (Uuid, f32)>,
    /// Lidas do banco (ou sem banco). Falso: recusa mudar.
    loaded: bool,
    retry_in: f32,
}

impl Companies {
    pub fn company_of(&self, who: CharacterId) -> Option<&Company> {
        let id = self.of.get(&who)?;
        self.list.iter().find(|company| company.id == *id)
    }

    pub fn id_of(&self, who: CharacterId) -> Option<Uuid> {
        self.of.get(&who).copied()
    }

    pub fn by_id(&self, id: Uuid) -> Option<&Company> {
        self.list.iter().find(|company| company.id == id)
    }

    pub fn same_company(&self, a: CharacterId, b: CharacterId) -> bool {
        matches!((self.of.get(&a), self.of.get(&b)), (Some(x), Some(y)) if x == y)
    }

    /// Todos os membros de todas as companhias (para a party saber quem é
    /// aliado).
    pub fn membership(&self) -> HashMap<CharacterId, Uuid> {
        self.of.clone()
    }

    fn replace_all(&mut self, list: Vec<Company>) {
        self.of = list
            .iter()
            .flat_map(|company| {
                company
                    .members
                    .iter()
                    .map(move |(member, ..)| (*member, company.id))
            })
            .collect();
        self.list = list;
        self.loaded = true;
    }

    /// Valida nome e tag; não mexe em nada.
    fn check_found(
        &self,
        founder: CharacterId,
        name: &str,
    ) -> Result<(String, String), &'static str> {
        if !self.loaded {
            return Err("Registro das companhias indisponível; tente daqui a pouco.");
        }
        if self.of.contains_key(&founder) {
            return Err("Você já está numa companhia.");
        }
        let name = clean_name(name)?;
        let lower = name.to_lowercase();
        if self
            .list
            .iter()
            .any(|company| company.name.to_lowercase() == lower)
        {
            return Err("Já existe uma companhia com esse nome.");
        }
        let tag = free_tag(&name, |tag| {
            self.list.iter().any(|company| company.tag == tag)
        })
        .ok_or("Sem tag livre para esse nome: tente outro.")?;
        Ok((name, tag))
    }

    fn found(
        &mut self,
        founder: CharacterId,
        founder_name: String,
        name: String,
        tag: String,
    ) -> Company {
        let company = Company {
            id: Uuid::new_v4(),
            name,
            tag,
            members: vec![(founder, founder_name, true)],
        };
        self.of.insert(founder, company.id);
        self.invites.remove(&founder);
        self.list.push(company.clone());
        company
    }

    fn invite(&mut self, from: CharacterId, to: CharacterId, now: f32) -> Result<(), &'static str> {
        if !self.loaded {
            return Err("Registro das companhias indisponível; tente daqui a pouco.");
        }
        let company = self.company_of(from).ok_or("Você não tem companhia.")?;
        if company.members.len() >= MAX_MEMBERS {
            return Err("Companhia cheia.");
        }
        if self.of.contains_key(&to) {
            return Err("Esse capitão já tem companhia.");
        }
        let id = company.id;
        self.invites.insert(to, (id, now + INVITE_SECS));
        Ok(())
    }

    fn pending_invite(&self, who: CharacterId, now: f32) -> Option<Uuid> {
        self.invites
            .get(&who)
            .filter(|(_, expires)| *expires > now)
            .map(|(id, _)| *id)
    }

    /// Aceita (devolve a companhia para gravar) ou recusa.
    fn answer(
        &mut self,
        who: CharacterId,
        name: String,
        accept: bool,
        now: f32,
    ) -> Result<Option<Company>, &'static str> {
        let id = self
            .pending_invite(who, now)
            .ok_or("Nenhum convite de companhia pendente.")?;
        self.invites.remove(&who);
        if !accept {
            return Ok(None);
        }
        if !self.loaded {
            return Err("Registro das companhias indisponível; tente daqui a pouco.");
        }
        if self.of.contains_key(&who) {
            return Err("Você já está numa companhia.");
        }
        let company = self
            .list
            .iter_mut()
            .find(|company| company.id == id)
            .ok_or("A companhia não existe mais.")?;
        if company.members.len() >= MAX_MEMBERS {
            return Err("Companhia cheia.");
        }
        company.members.push((who, name, false));
        self.of.insert(who, id);
        Ok(Some(company.clone()))
    }

    /// Sai; líder que sai passa a liderança ao mais antigo. Devolve a
    /// companhia para gravar ou o id para apagar (ficou vazia).
    fn leave(&mut self, who: CharacterId) -> Result<Result<Company, Uuid>, &'static str> {
        if !self.loaded {
            return Err("Registro das companhias indisponível; tente daqui a pouco.");
        }
        let id = self
            .of
            .remove(&who)
            .ok_or("Você não está numa companhia.")?;
        let Some(index) = self.list.iter().position(|company| company.id == id) else {
            return Err("Você não está numa companhia.");
        };
        let company = &mut self.list[index];
        company.members.retain(|(member, ..)| *member != who);
        if company.members.is_empty() {
            self.list.remove(index);
            return Ok(Err(id));
        }
        if !company.members.iter().any(|(_, _, leader)| *leader) {
            company.members[0].2 = true;
        }
        Ok(Ok(company.clone()))
    }

    /// Desfaz uma mudança que o banco recusou.
    fn restore(&mut self, before: Vec<Company>) {
        self.replace_all(before);
    }
}

/// Nome: 3 a 20 letras (espaço e apóstrofo valem), espaços colapsados.
fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = name.chars().count();
    if !(3..=20).contains(&count) {
        return Err("Nome de companhia: de 3 a 20 letras.");
    }
    if !name
        .chars()
        .all(|c| c.is_alphabetic() || c == ' ' || c == '\'')
    {
        return Err("Nome de companhia: só letras, espaço e apóstrofo.");
    }
    Ok(name)
}

/// Tag das iniciais (sem "do", "da"…), 2 a 4 letras maiúsculas; ocupada,
/// troca a última letra por um número.
fn free_tag(name: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    const SMALL: [&str; 7] = ["do", "da", "de", "dos", "das", "e", "o"];
    let words: Vec<&str> = name
        .split(' ')
        .filter(|word| !SMALL.contains(&word.to_lowercase().as_str()))
        .collect();
    let base: String = if words.len() >= 2 {
        words
            .iter()
            .filter_map(|word| word.chars().next())
            .take(4)
            .collect()
    } else {
        name.chars().filter(|c| c.is_alphabetic()).take(3).collect()
    };
    let base = base.to_uppercase();
    if base.chars().count() < 2 {
        return None;
    }
    if !taken(&base) {
        return Some(base);
    }
    let stem: String = base.chars().take(3).collect();
    (2..=9)
        .map(|n| format!("{stem}{n}"))
        .find(|tag| !taken(tag))
}

pub fn install(app: &mut App) {
    app.init_resource::<Companies>();
}

/// Lê as companhias no boot (e de novo, se falhou).
pub fn load_companies(
    time: Res<Time>,
    store: Res<StoreHandle>,
    mut companies: ResMut<Companies>,
    mut parties: ResMut<crate::party::Parties>,
) {
    if companies.loaded {
        return;
    }
    companies.retry_in -= time.delta_secs();
    if companies.retry_in > 0.0 {
        return;
    }
    let Some(store) = store.0.as_ref() else {
        companies.replace_all(Vec::new());
        return;
    };
    match store.load_companies() {
        Ok(list) => {
            info!(count = list.len(), "companhias carregadas");
            companies.replace_all(list);
            parties.set_companies(companies.membership());
        }
        Err(error) => {
            warn!(%error, "companhias não carregaram; mudanças recusadas até ler");
            companies.retry_in = RETRY_LOAD_SECS;
        }
    }
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

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn handle_company_intents(
    time: Res<Time>,
    mut connection_manager: ResMut<ConnectionManager>,
    (mut creates, mut invites, mut answers, mut leaves): (
        EventReader<ServerReceiveMessage<CreateCompany>>,
        EventReader<ServerReceiveMessage<CompanyInvite>>,
        EventReader<ServerReceiveMessage<CompanyAnswer>>,
        EventReader<ServerReceiveMessage<LeaveCompany>>,
    ),
    names: Res<CaptainNames>,
    store: Res<StoreHandle>,
    dev: Res<DevItems>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut companies: ResMut<Companies>,
    mut parties: ResMut<crate::party::Parties>,
    ships: Query<&ServerShip>,
) {
    let now = time.elapsed_secs();
    let by_client = |client: ClientId| ships.iter().find(|ship| ship.client_id == Some(client));
    let mut changed = false;
    // Grava no banco; se recusar, volta ao estado anterior.
    let persist = |companies: &mut Companies,
                   before: Vec<Company>,
                   change: Result<Company, Uuid>|
     -> Result<(), &'static str> {
        let Some(store) = store.0.as_ref() else {
            return Ok(());
        };
        let saved = match &change {
            Ok(company) => store.save_company(company),
            Err(id) => store.delete_company(*id),
        };
        saved.map_err(|error| {
            warn!(%error, "companhia não gravou; desfeito");
            companies.restore(before);
            "O registro das companhias falhou; nada mudou."
        })
    };

    for event in creates.read() {
        let Some(me) = by_client(event.from()) else {
            continue;
        };
        let VesselPresence::Docked(region) = me.presence else {
            notify(
                &mut connection_manager,
                me.client_id,
                "Companhia se funda no porto: atraque primeiro.",
            );
            continue;
        };
        let (name, tag) = match companies.check_found(me.character, &event.message().name) {
            Ok(found) => found,
            Err(reason) => {
                notify(&mut connection_manager, me.client_id, reason);
                continue;
            }
        };
        // Dev (teste ao vivo): MARVYR_DEV_COMPANY_FREE=1 funda sem custo,
        // nunca em produção.
        let free = std::env::var_os("MARVYR_DEV_COMPANY_FREE").is_some()
            && !std::env::var("MARVYR_ENV").is_ok_and(|env| env == "production");
        // Escambo: madeira e minério do armazém; falhou o segundo, devolve
        // o primeiro.
        if !free
            && market
                .consume_from_storage(me.character, region, dev.ore, FOUND_ORE)
                .is_err()
        {
            notify(
                &mut connection_manager,
                me.client_id,
                format!("Fundar custa {FOUND_TIMBER} Madeira e {FOUND_ORE} Minério no armazém."),
            );
            continue;
        }
        if !free
            && market
                .consume_from_storage(me.character, region, dev.timber, FOUND_TIMBER)
                .is_err()
        {
            market.grant_to_storage(me.character, region, dev.ore, FOUND_ORE, &dev.catalog);
            notify(
                &mut connection_manager,
                me.client_id,
                format!("Fundar custa {FOUND_TIMBER} Madeira e {FOUND_ORE} Minério no armazém."),
            );
            continue;
        }
        let before = companies.list.clone();
        let company = companies.found(me.character, names.of(me), name, tag);
        if let Err(reason) = persist(&mut companies, before, Ok(company.clone())) {
            if !free {
                market.grant_to_storage(me.character, region, dev.ore, FOUND_ORE, &dev.catalog);
                market.grant_to_storage(
                    me.character,
                    region,
                    dev.timber,
                    FOUND_TIMBER,
                    &dev.catalog,
                );
            }
            notify(&mut connection_manager, me.client_id, reason);
            continue;
        }
        info!(company = %company.name, tag = %company.tag, "companhia fundada");
        notify(
            &mut connection_manager,
            me.client_id,
            format!("Companhia {} [{}] fundada!", company.name, company.tag),
        );
        changed = true;
    }

    for event in invites.read() {
        let Some(me) = by_client(event.from()) else {
            continue;
        };
        let target = ships.iter().find(|ship| {
            ship.ship_id == event.message().target_ship_id
                && ship.client_id.is_some()
                && matches!(ship.presence, VesselPresence::Docked(_))
                && ship.presence == me.presence
        });
        let Some(target) = target else {
            notify(
                &mut connection_manager,
                me.client_id,
                "Só dá para convidar quem está atracado no mesmo porto.",
            );
            continue;
        };
        match companies.invite(me.character, target.character, now) {
            Ok(()) => {
                let company = companies
                    .company_of(me.character)
                    .map(|company| format!("{} [{}]", company.name, company.tag))
                    .unwrap_or_default();
                notify(
                    &mut connection_manager,
                    me.client_id,
                    format!("Convite enviado a {}", names.of(target)),
                );
                notify(
                    &mut connection_manager,
                    target.client_id,
                    format!(
                        "{} te chamou para a companhia {company}: veja a aba Companhia",
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
        let before = companies.list.clone();
        match companies.answer(me.character, names.of(me), accept, now) {
            Ok(Some(company)) => {
                if let Err(reason) = persist(&mut companies, before, Ok(company.clone())) {
                    notify(&mut connection_manager, me.client_id, reason);
                    continue;
                }
                notify(
                    &mut connection_manager,
                    me.client_id,
                    format!(
                        "Você entrou na companhia {} [{}]",
                        company.name, company.tag
                    ),
                );
                changed = true;
            }
            Ok(None) => notify(&mut connection_manager, me.client_id, "Convite recusado."),
            Err(reason) => notify(&mut connection_manager, me.client_id, reason),
        }
    }

    for event in leaves.read() {
        let Some(me) = by_client(event.from()) else {
            continue;
        };
        let before = companies.list.clone();
        match companies.leave(me.character) {
            Ok(change) => {
                if let Err(reason) = persist(&mut companies, before, change) {
                    notify(&mut connection_manager, me.client_id, reason);
                    continue;
                }
                notify(
                    &mut connection_manager,
                    me.client_id,
                    "Você saiu da companhia.",
                );
                changed = true;
            }
            Err(reason) => notify(&mut connection_manager, me.client_id, reason),
        }
    }
    if changed {
        parties.set_companies(companies.membership());
    }
}

/// Manda a companhia, o convite, quem dá para convidar e as frentes de
/// guerra a cada capitão.
pub fn broadcast_companies(
    time: Res<Time>,
    mut clock: Local<f32>,
    names: Res<CaptainNames>,
    companies: Res<Companies>,
    lords: Res<crate::territory::PortLords>,
    ships: Query<&ServerShip>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    *clock += time.delta_secs();
    if *clock < UPDATE_EVERY_SECS {
        return;
    }
    *clock = 0.0;
    let now = time.elapsed_secs();
    let online: Vec<CharacterId> = ships
        .iter()
        .filter(|ship| ship.client_id.is_some())
        .map(|ship| ship.character)
        .collect();
    for ship in &ships {
        let Some(client) = ship.client_id else {
            continue;
        };
        let company = companies.company_of(ship.character);
        let docked_here = match (company, ship.presence) {
            (Some(company), VesselPresence::Docked(_))
                if company
                    .members
                    .iter()
                    .any(|(member, _, leader)| *member == ship.character && *leader) =>
            {
                ships
                    .iter()
                    .filter(|other| {
                        other.ship_id != ship.ship_id
                            && other.client_id.is_some()
                            && other.presence == ship.presence
                            && companies.id_of(other.character).is_none()
                    })
                    .map(|other| (other.ship_id, names.of(other)))
                    .collect()
            }
            _ => Vec::new(),
        };
        let update = CompanyUpdate {
            name: company.map(|c| c.name.clone()).unwrap_or_default(),
            tag: company.map(|c| c.tag.clone()).unwrap_or_default(),
            leader: company.is_some_and(|c| {
                c.members
                    .iter()
                    .any(|(member, _, leader)| *member == ship.character && *leader)
            }),
            members: company
                .map(|c| {
                    c.members
                        .iter()
                        .map(|(member, name, leader)| CompanyMember {
                            name: name.clone(),
                            online: online.contains(member),
                            leader: *leader,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ports: company
                .map(|c| lords.ports_of(&crate::territory::Holder::Company(c.id)))
                .unwrap_or_default(),
            invite_from: companies
                .pending_invite(ship.character, now)
                .and_then(|id| companies.by_id(id))
                .map(|c| format!("{} [{}]", c.name, c.tag)),
            docked_here,
            wars: lords.fronts_for(&crate::territory::holder_of(&companies, ship.character)),
        };
        let _ = connection_manager.send_message::<ReliableChannel, _>(client, &update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Companies {
        let mut companies = Companies::default();
        companies.replace_all(Vec::new());
        companies
    }

    #[test]
    fn names_and_tags() {
        assert_eq!(
            clean_name("  Irmandade   do Coral "),
            Ok(String::from("Irmandade do Coral"))
        );
        assert!(clean_name("ab").is_err());
        assert!(clean_name("Piratas 123").is_err());
        assert_eq!(
            free_tag("Irmandade do Coral", |_| false).as_deref(),
            Some("IC")
        );
        assert_eq!(free_tag("Tubarões", |_| false).as_deref(), Some("TUB"));
        assert_eq!(
            free_tag("Irmandade do Coral", |tag| tag == "IC").as_deref(),
            Some("IC2")
        );
    }

    #[test]
    fn found_invite_join_and_leave() {
        let mut companies = loaded();
        let (a, b, c) = (CharacterId::new(), CharacterId::new(), CharacterId::new());
        let (name, tag) = companies.check_found(a, "Irmandade do Coral").unwrap();
        let founded = companies.found(a, String::from("A"), name, tag);
        assert!(
            companies.check_found(b, "irmandade do coral").is_err(),
            "nome repetido"
        );
        assert!(
            companies.check_found(a, "Outra Coisa").is_err(),
            "já tem companhia"
        );
        companies.invite(a, b, 0.0).unwrap();
        assert!(
            companies.invite(b, c, 0.0).is_err(),
            "sem companhia não convida"
        );
        let joined = companies
            .answer(b, String::from("B"), true, 1.0)
            .unwrap()
            .unwrap();
        assert_eq!(joined.members.len(), 2);
        assert!(companies.same_company(a, b));
        // Líder sai: B herda; B sai: a companhia some.
        let after = companies.leave(a).unwrap().unwrap();
        assert!(after.members[0].2, "liderança passou");
        assert_eq!(companies.leave(b).unwrap(), Err(founded.id));
        assert!(companies.company_of(b).is_none());
    }

    #[test]
    fn unread_registry_refuses_changes() {
        let companies = Companies::default();
        assert!(companies
            .check_found(CharacterId::new(), "Irmandade do Coral")
            .is_err());
    }

    #[test]
    fn stale_invites_expire() {
        let mut companies = loaded();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        let (name, tag) = companies.check_found(a, "Corsarios").unwrap();
        companies.found(a, String::from("A"), name, tag);
        companies.invite(a, b, 0.0).unwrap();
        assert!(companies
            .answer(b, String::from("B"), true, INVITE_SECS + 1.0)
            .is_err());
    }
}
