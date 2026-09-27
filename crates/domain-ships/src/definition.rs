use marvyr_domain_items::EquipmentSlot;
use marvyr_shared::ids::ShipDefinitionId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShipKind {
    SmallMerchant, // 3 tipos do vertical slice
    Patrol,
    Corsair,
    /// v58: bergantim — caçador rápido, porão curto.
    Brig,
    /// v58: galeão — lento, casco grosso, porão enorme.
    Galleon,
    /// v58: bombarda — canhão pesado e leque mais forte, manobra mal.
    Bombard,
}

impl ShipKind {
    pub const ALL: [ShipKind; 6] = [
        ShipKind::SmallMerchant,
        ShipKind::Patrol,
        ShipKind::Corsair,
        ShipKind::Brig,
        ShipKind::Galleon,
        ShipKind::Bombard,
    ];

    /// v58: bônus do casco na skill Z (a bombarda nasceu para o leque).
    pub fn fan_bonus(self) -> f32 {
        match self {
            ShipKind::Bombard => 1.5,
            _ => 1.0,
        }
    }

    /// Nome do casco para o jogador (PT-BR; o client traduz).
    pub fn name(self) -> &'static str {
        match self {
            ShipKind::SmallMerchant => "Mercante",
            ShipKind::Patrol => "Patrulha",
            ShipKind::Corsair => "Corsário",
            ShipKind::Brig => "Bergantim",
            ShipKind::Galleon => "Galeão",
            ShipKind::Bombard => "Bombarda",
        }
    }

    /// v42: título de quem leva a maestria deste casco ao máximo.
    pub fn master_title(self) -> &'static str {
        match self {
            ShipKind::SmallMerchant => "Mestre do Mercante",
            ShipKind::Patrol => "Mestre da Patrulha",
            ShipKind::Corsair => "Mestre do Corsário",
            ShipKind::Brig => "Mestre do Bergantim",
            ShipKind::Galleon => "Mestre do Galeão",
            ShipKind::Bombard => "Mestre da Bombarda",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSpec {
    pub kind: EquipmentSlot,
    pub accepts_tag: Option<String>, // tag opcional de ItemDefinition; None = aceita qualquer
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDefinition {
    pub id: ShipDefinitionId,
    pub kind: ShipKind,
    pub display_name: String,
    pub slots: Vec<SlotSpec>,
    pub cargo_capacity: u32, // em unidades de peso
    pub base_speed: f32,     // m/s
    pub base_turn_rate: f32, // rad/s
    pub base_hp: u32,
    pub base_weapon_damage: u32,
    pub base_weapon_range: f32, // metros
}

impl ShipDefinition {
    pub fn slot_count(&self, kind: EquipmentSlot) -> usize {
        self.slots.iter().filter(|s| s.kind == kind).count()
    }

    fn slots_of(kind: EquipmentSlot) -> Vec<SlotSpec> {
        vec![SlotSpec {
            kind,
            accepts_tag: None,
        }]
    }

    /// Transportador (PRD §12): maior carga, suficiente para tentar fugir.
    /// Client e server usam a mesma definição para que stats e movimento
    /// batam nos dois lados.
    pub fn small_merchant() -> Self {
        Self {
            id: ShipDefinitionId::new(),
            kind: ShipKind::SmallMerchant,
            display_name: String::from("Small Merchant"),
            slots: Self::slots_of(EquipmentSlot::Hull)
                .into_iter()
                .chain(Self::slots_of(EquipmentSlot::Sail))
                .chain(Self::slots_of(EquipmentSlot::Weapon))
                .collect(),
            cargo_capacity: 100,
            base_speed: 30.0,
            base_turn_rate: 1.0,
            base_hp: 100,
            base_weapon_damage: 20,
            base_weapon_range: 210.0,
        }
    }

    /// Compat: nome antigo do placeholder do SmallMerchant (MF-022 trouxe o
    /// catálogo real; o nome antigo sobrevive no dev respawn).
    pub fn small_merchant_placeholder() -> Self {
        Self::small_merchant()
    }

    /// Controle de área e escolta (PRD §13): casco grosso, carga média.
    pub fn patrol() -> Self {
        Self {
            id: ShipDefinitionId::new(),
            kind: ShipKind::Patrol,
            display_name: String::from("Patrol"),
            slots: Self::slots_of(EquipmentSlot::Hull)
                .into_iter()
                .chain(Self::slots_of(EquipmentSlot::Sail))
                .chain(Self::slots_of(EquipmentSlot::Weapon))
                .collect(),
            cargo_capacity: 70,
            base_speed: 27.0,
            base_turn_rate: 0.9,
            base_hp: 160,
            base_weapon_damage: 20,
            base_weapon_range: 210.0,
        }
    }

    /// Interceptador (PRD §14): velocidade e pressão ofensiva, porão curto.
    pub fn corsair() -> Self {
        Self {
            id: ShipDefinitionId::new(),
            kind: ShipKind::Corsair,
            display_name: String::from("Corsair"),
            slots: Self::slots_of(EquipmentSlot::Hull)
                .into_iter()
                .chain(Self::slots_of(EquipmentSlot::Sail))
                .chain(Self::slots_of(EquipmentSlot::Weapon))
                .collect(),
            cargo_capacity: 40,
            base_speed: 40.0,
            base_turn_rate: 1.2,
            base_hp: 70,
            base_weapon_damage: 25,
            base_weapon_range: 240.0,
        }
    }
}

impl ShipDefinition {
    fn three_slots() -> Vec<SlotSpec> {
        [
            EquipmentSlot::Hull,
            EquipmentSlot::Sail,
            EquipmentSlot::Weapon,
        ]
        .into_iter()
        .flat_map(Self::slots_of)
        .collect()
    }

    /// v58: bergantim — o caçador: rápido e ágil, porão curto.
    pub fn brig() -> Self {
        Self {
            id: ShipDefinitionId::new(),
            kind: ShipKind::Brig,
            display_name: String::from("Brig"),
            slots: Self::three_slots(),
            cargo_capacity: 50,
            base_speed: 44.0,
            base_turn_rate: 1.35,
            base_hp: 85,
            base_weapon_damage: 22,
            base_weapon_range: 230.0,
        }
    }

    /// v58: galeão — o caminhão blindado do mercador.
    pub fn galleon() -> Self {
        Self {
            id: ShipDefinitionId::new(),
            kind: ShipKind::Galleon,
            display_name: String::from("Galleon"),
            slots: Self::three_slots(),
            cargo_capacity: 180,
            base_speed: 23.0,
            base_turn_rate: 0.7,
            base_hp: 220,
            base_weapon_damage: 18,
            base_weapon_range: 210.0,
        }
    }

    /// v58: bombarda — canhão pesado; o leque bate 50% mais.
    pub fn bombard() -> Self {
        Self {
            id: ShipDefinitionId::new(),
            kind: ShipKind::Bombard,
            display_name: String::from("Bombard"),
            slots: Self::three_slots(),
            cargo_capacity: 45,
            base_speed: 28.0,
            base_turn_rate: 0.95,
            base_hp: 110,
            base_weapon_damage: 30,
            base_weapon_range: 220.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> ShipDefinition {
        ShipDefinition {
            id: ShipDefinitionId::new(),
            kind: ShipKind::SmallMerchant,
            display_name: String::new(),
            slots: vec![
                SlotSpec {
                    kind: EquipmentSlot::Hull,
                    accepts_tag: Some("hull".to_owned()),
                },
                SlotSpec {
                    kind: EquipmentSlot::Hull,
                    accepts_tag: None,
                },
                SlotSpec {
                    kind: EquipmentSlot::Sail,
                    accepts_tag: None,
                },
                SlotSpec {
                    kind: EquipmentSlot::Weapon,
                    accepts_tag: None,
                },
            ],
            cargo_capacity: 100,
            base_speed: 5.0,
            base_turn_rate: 1.0,
            base_hp: 100,
            base_weapon_damage: 20,
            base_weapon_range: 50.0,
        }
    }

    #[test]
    fn slot_count_counts_each_kind() {
        let def = def();
        assert_eq!(def.slot_count(EquipmentSlot::Hull), 2);
        assert_eq!(def.slot_count(EquipmentSlot::Sail), 1);
        assert_eq!(def.slot_count(EquipmentSlot::Weapon), 1);
        assert_eq!(def.slot_count(EquipmentSlot::Aux), 0);
    }
}
