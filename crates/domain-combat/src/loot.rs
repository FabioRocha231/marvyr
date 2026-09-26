//! Resolução de full loot (PRD §22-§26, MF-013) e política de wreck (§26, MF-014).
//!
//! Regra de ouro do §22: full loot significa **o derrotado perdeu tudo** —
//! não que o vencedor receba tudo. Parte desaparece; o que sobrevive vira
//! `Wreck`. A resolução é determinística: mesmo `DestructionEventId` + mesma
//! entrada → mesmo resultado (§24), auditável sem Bevy.

use marvyr_domain_items::instance::ItemInstance;
use marvyr_domain_items::location::{Custody, ItemLocation};
use marvyr_shared::ids::{DestructionEventId, ItemDefinitionId, ItemInstanceId, WreckId};
use serde::{Deserialize, Serialize};

/// Taxas de sobrevivência por categoria (PRD §23: tuning inicial — casco é
/// 100% perda e não entra aqui; equipamento ~50%; carga ~80%).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LootPolicy {
    /// Fração de equipamentos equipados que sobrevivem para o wreck, por item.
    pub equipment_survival_rate: f32,
    /// Fração da carga que sobrevive para o wreck, por unidade.
    pub cargo_survival_rate: f32,
}

impl Default for LootPolicy {
    fn default() -> Self {
        Self {
            equipment_survival_rate: 0.5,
            cargo_survival_rate: 0.8,
        }
    }
}

/// Item que sobreviveu ao naufrágio (sem id de instância: o servidor atribui
/// ids reais ao materializar o wreck — o resultado continua reproduzível).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurvivorItem {
    pub definition: ItemDefinitionId,
    pub quantity: u32,
    pub durability: Option<u16>,
    /// Afixos da peça (Mágica/Rara) — sobrevivem junto com ela.
    #[serde(default)]
    pub quality: Option<marvyr_domain_items::Quality>,
}

/// Resultado da resolução (PRD §25). `destroyed_ship` é sempre `true`: casco
/// nunca aparece em wreck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DestructionOutcome {
    pub destroyed_ship: bool,
    pub destroyed_items: Vec<SurvivorItem>,
    pub wreck_items: Vec<SurvivorItem>,
}

/// Resolve o naufrágio. Equipamento: rolagem por item, com seed derivada do
/// `DestructionEventId`. Carga: divisão proporcional por unidade.
pub fn resolve_ship_destruction(
    destruction: DestructionEventId,
    equipment: &[ItemInstance],
    cargo: &[ItemInstance],
    policy: &LootPolicy,
) -> DestructionOutcome {
    let mut rng = seed_from(destruction);

    let mut wreck_items = Vec::new();
    let mut destroyed_items = Vec::new();

    for piece in equipment {
        let survivor = SurvivorItem {
            definition: piece.definition,
            quantity: 1,
            durability: None,
            quality: piece.quality.clone(),
        };
        if roll_survives(&mut rng, policy.equipment_survival_rate) {
            wreck_items.push(survivor);
        } else {
            destroyed_items.push(survivor);
        }
    }

    for item in cargo {
        let surviving = (item.quantity as f32 * policy.cargo_survival_rate).floor() as u32;
        if surviving > 0 {
            wreck_items.push(SurvivorItem {
                definition: item.definition,
                quantity: surviving,
                durability: item.durability,
                quality: item.quality.clone(),
            });
        }
        let destroyed = item.quantity - surviving;
        if destroyed > 0 {
            destroyed_items.push(SurvivorItem {
                definition: item.definition,
                quantity: destroyed,
                durability: item.durability,
                quality: item.quality.clone(),
            });
        }
    }

    DestructionOutcome {
        destroyed_ship: true,
        destroyed_items,
        wreck_items,
    }
}

/// splitmix64: PRNG minúsculo e determinístico — nada de RNG dependente de
/// client (PRD §24).
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn roll_survives(state: &mut u64, rate: f32) -> bool {
    let roll = (splitmix64(state) % 10_000) as f32 / 10_000.0;
    roll < rate
}

fn seed_from(destruction: DestructionEventId) -> u64 {
    let mixed = destruction.0.as_u128() as u64;
    let mut state = mixed ^ (mixed >> 32);
    splitmix64(&mut state)
}

/// Política de vida do wreck (PRD §26): 45s exclusivos ao killer, depois
/// free-for-all, desaparece em 5 minutos. Números vivem na configuração do
/// servidor; as regras de janela vivem aqui.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WreckPolicy {
    pub exclusive_window_secs: f32,
    pub total_lifetime_secs: f32,
}

impl Default for WreckPolicy {
    fn default() -> Self {
        Self {
            exclusive_window_secs: 45.0,
            total_lifetime_secs: 300.0,
        }
    }
}

/// Quem pode saquear agora. `requester`/`exclusive_looter` são chaves opacas
/// do dono (MF-035: o servidor amarra com `CharacterId` — a janela é do
/// personagem, não da sessão/conexão).
pub fn can_loot(
    elapsed_secs: f32,
    policy: &WreckPolicy,
    requester: marvyr_shared::ids::CharacterId,
    exclusive_looter: Option<marvyr_shared::ids::CharacterId>,
) -> bool {
    match exclusive_looter {
        Some(killer) if elapsed_secs < policy.exclusive_window_secs => requester == killer,
        _ => true,
    }
}

pub fn is_expired(elapsed_secs: f32, policy: &WreckPolicy) -> bool {
    elapsed_secs >= policy.total_lifetime_secs
}

/// Baú do wreck: itens em custódia `ItemLocation::Wreck`. O servidor materializa
/// os sobreviventes aqui e transfere para o porão do saqueador (MF-015).
#[derive(Debug, Clone, PartialEq)]
pub struct WreckChest {
    wreck: WreckId,
    items: Vec<Custody>,
}

impl WreckChest {
    pub fn new(wreck: WreckId) -> Self {
        Self {
            wreck,
            items: Vec::new(),
        }
    }

    pub fn wreck(&self) -> WreckId {
        self.wreck
    }

    pub fn items(&self) -> &[Custody] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Drena o baú de uma vez (§70: a operação de loot é a única vencedora —
    /// o segundo saque encontra o baú vazio e falha, sem duplicar item).
    pub fn drain(&mut self) -> Vec<Custody> {
        std::mem::take(&mut self.items)
    }

    /// Materializa um sobrevivente como instância real no baú.
    pub fn insert(&mut self, survivor: SurvivorItem, instance_id: ItemInstanceId) {
        self.items.push(Custody::new(
            ItemInstance {
                id: instance_id,
                definition: survivor.definition,
                quantity: survivor.quantity,
                durability: survivor.durability,
                quality: survivor.quality,
            },
            ItemLocation::Wreck(self.wreck),
        ));
    }

    /// Esvazia o baú e devolve as custódias para transferência.
    pub fn drain_all(self) -> Vec<Custody> {
        self.items
    }
}

#[cfg(test)]
mod tests {
    use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};

    use super::*;

    fn event(seed: u64) -> DestructionEventId {
        // Uuid determinístico a partir do seed para testes reproduzíveis.
        let bytes = seed.to_le_bytes();
        let mut full = [0u8; 16];
        full[..8].copy_from_slice(&bytes);
        full[8..].copy_from_slice(&bytes);
        DestructionEventId(uuid::Uuid::from_bytes(full))
    }

    fn timber() -> ItemDefinitionId {
        ItemDefinitionId::new()
    }

    fn piece() -> ItemInstance {
        ItemInstance::new_equipment(ItemInstanceId::new(), ItemDefinitionId::new(), 100)
    }

    fn cargo_item(def: ItemDefinitionId, quantity: u32) -> ItemInstance {
        ItemInstance::new_resource(ItemInstanceId::new(), def, quantity)
    }

    #[test]
    fn surviving_piece_keeps_its_affixes() {
        let quality = marvyr_domain_items::roll_quality(marvyr_domain_items::Rarity::Rare, 9);
        let rare = ItemInstance {
            quality: quality.clone(),
            ..piece()
        };
        let policy = LootPolicy {
            equipment_survival_rate: 1.0,
            ..LootPolicy::default()
        };
        let outcome = resolve_ship_destruction(event(2), &[rare], &[], &policy);
        let mut chest = WreckChest::new(WreckId::new());
        chest.insert(outcome.wreck_items[0].clone(), ItemInstanceId::new());
        assert_eq!(chest.items()[0].instance.quality, quality);
    }

    #[test]
    fn empty_ship_sinks_with_nothing_in_wreck() {
        let outcome = resolve_ship_destruction(event(1), &[], &[], &LootPolicy::default());

        assert!(outcome.destroyed_ship);
        assert!(outcome.wreck_items.is_empty());
        assert!(outcome.destroyed_items.is_empty());
    }

    #[test]
    fn cargo_survives_proportionally() {
        let def = timber();
        let outcome = resolve_ship_destruction(
            event(7),
            &[],
            &[cargo_item(def, 10)],
            &LootPolicy::default(), // 80%
        );

        assert_eq!(outcome.wreck_items.len(), 1);
        assert_eq!(outcome.wreck_items[0].quantity, 8);
        assert_eq!(outcome.destroyed_items[0].quantity, 2);
    }

    #[test]
    fn proportional_rounding_never_exceeds_original() {
        let def = timber();
        let outcome =
            resolve_ship_destruction(event(3), &[], &[cargo_item(def, 5)], &LootPolicy::default());

        let wrecked: u32 = outcome.wreck_items.iter().map(|i| i.quantity).sum();
        let destroyed: u32 = outcome.destroyed_items.iter().map(|i| i.quantity).sum();
        assert_eq!(wrecked + destroyed, 5);
        assert_eq!(wrecked, 4);
    }

    #[test]
    fn same_seed_same_outcome() {
        let def = timber();
        let equipment: Vec<ItemInstance> = (0..5).map(|_| piece()).collect();
        let cargo = [cargo_item(def, 13)];

        let a = resolve_ship_destruction(event(42), &equipment, &cargo, &LootPolicy::default());
        let b = resolve_ship_destruction(event(42), &equipment, &cargo, &LootPolicy::default());

        assert_eq!(a, b);
    }

    #[test]
    fn outcome_conserves_every_unit() {
        let def = timber();
        let equipment: Vec<ItemInstance> = (0..20).map(|_| piece()).collect();
        let outcome = resolve_ship_destruction(
            event(99),
            &equipment,
            &[cargo_item(def, 17)],
            &LootPolicy::default(),
        );

        let wrecked: u32 = outcome.wreck_items.iter().map(|i| i.quantity).sum();
        let destroyed: u32 = outcome.destroyed_items.iter().map(|i| i.quantity).sum();
        // 20 equipamentos (1 unidade cada) + 17 de carga = 37 unidades no total.
        assert_eq!(wrecked + destroyed, 37);
    }

    #[test]
    fn zero_rate_destroys_all_cargo() {
        let def = timber();
        let policy = LootPolicy {
            cargo_survival_rate: 0.0,
            ..LootPolicy::default()
        };
        let outcome = resolve_ship_destruction(event(5), &[], &[cargo_item(def, 10)], &policy);

        assert!(outcome.wreck_items.is_empty());
        assert_eq!(outcome.destroyed_items[0].quantity, 10);
    }

    #[test]
    fn wreck_window_is_exclusive_to_killer_then_ffa() {
        use marvyr_shared::ids::CharacterId;
        let policy = WreckPolicy::default();
        let killer = CharacterId::new();
        let other = CharacterId::new();

        // Dentro da janela: só o killer.
        assert!(can_loot(10.0, &policy, killer, Some(killer)));
        assert!(!can_loot(10.0, &policy, other, Some(killer)));
        // Janela passou: free-for-all.
        assert!(can_loot(50.0, &policy, other, Some(killer)));
        // Sem exclusivo: qualquer um.
        assert!(can_loot(1.0, &policy, other, None));
    }

    #[test]
    fn wreck_expires_after_total_lifetime() {
        let policy = WreckPolicy::default();
        assert!(!is_expired(299.0, &policy));
        assert!(is_expired(300.0, &policy));
    }

    #[test]
    fn chest_stamps_wreck_location_and_drains() {
        let wreck = WreckId::new();
        let mut chest = WreckChest::new(wreck);
        let def = timber();
        chest.insert(
            SurvivorItem {
                definition: def,
                quantity: 8,
                durability: None,
                quality: None,
            },
            ItemInstanceId::new(),
        );

        assert_eq!(chest.items()[0].location, ItemLocation::Wreck(wreck));
        let drained = chest.drain_all();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].location, ItemLocation::Wreck(wreck));
    }
}
