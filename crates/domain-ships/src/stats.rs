use marvyr_domain_items::{AffixTotals, CatalogError, ItemCatalog};
use serde::{Deserialize, Serialize};

use crate::components::EquippedComponents;
use crate::definition::ShipDefinition;

/// Erros de cálculo de stats. Toda falha vem de lookup fail-closed no
/// catálogo de itens.
pub type StatsError = CatalogError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipStats {
    pub speed: f32,
    pub turn_rate: f32,
    pub max_hp: u32,
    pub cargo_capacity: u32,
    pub weapon_damage: u32,
    pub weapon_range: f32,
    /// Multiplicador do tempo de recarga (afixo de Recarga; 1 = normal).
    #[serde(default = "full_reload")]
    pub reload_factor: f32,
}

fn full_reload() -> f32 {
    1.0
}

/// Afixos de recarga somados não passam disto: canhão que nunca esfria
/// quebraria o combate.
const MAX_RELOAD_CUT_PCT: i32 = 40;
/// Gema que atrasa a recarga (Rubi) também tem teto: nunca mais que o dobro.
const MAX_RELOAD_SLOW_PCT: i32 = 100;

/// Calcula os stats do navio a partir da definição e dos componentes
/// equipados. Falha fechada: componente com definição ausente no catálogo
/// ou que não é equipamento é erro, nunca assumido como zero.
pub fn compute_ship_stats(
    def: &ShipDefinition,
    equipped: &EquippedComponents,
    catalog: &ItemCatalog,
) -> Result<ShipStats, StatsError> {
    let mut damage: i32 = def.base_weapon_damage as i32;
    let mut speed: f32 = def.base_speed;
    let mut cargo: i32 = def.cargo_capacity as i32;
    let mut hp: i32 = def.base_hp as i32;
    let mut range: f32 = def.base_weapon_range;
    let mut affixes = AffixTotals::default();

    for comp in equipped
        .hull
        .iter()
        .chain(equipped.sail.iter())
        .chain(equipped.weapon.iter())
        .chain(equipped.aux.iter())
    {
        let mods = catalog.equipment_stats(comp.item_definition)?;
        damage += mods.damage;
        speed += mods.speed as f32 * 0.01;
        cargo += mods.cargo;
        hp += mods.hp;
        range += mods.range as f32 * 0.01;
        for affix in &comp.affixes {
            affixes.add(*affix);
        }
    }
    let pct = |value: f32, pct: i32| value * (100 + pct) as f32 / 100.0;
    damage += affixes.damage;
    hp += affixes.hull;
    cargo += affixes.cargo;

    Ok(ShipStats {
        speed: pct(speed, affixes.speed_pct).max(0.0),
        turn_rate: pct(def.base_turn_rate, affixes.turn_pct).max(0.0),
        max_hp: hp.max(0) as u32,
        cargo_capacity: cargo.max(0) as u32,
        weapon_damage: damage.max(0) as u32,
        weapon_range: pct(range, affixes.range_pct).max(0.0),
        reload_factor: (100
            - affixes
                .reload_pct
                .clamp(-MAX_RELOAD_SLOW_PCT, MAX_RELOAD_CUT_PCT)) as f32
            / 100.0,
    })
}

/// HP depois de o máximo mudar (casco equipado, talento aprendido): mantém
/// a fração de casco inteiro, arredondando para baixo. Casco cheio continua
/// cheio (instalar um casco reforçado soma os pontos dele); avariado continua
/// avariado na mesma proporção — trocar peça no porto não conserta nada, e
/// equipar/desequipar em ciclo nunca ganha HP.
pub fn rescale_hp(hp: u32, old_max: u32, new_max: u32) -> u32 {
    if old_max == 0 {
        return hp.min(new_max);
    }
    let scaled = u64::from(hp.min(old_max)) * u64::from(new_max) / u64::from(old_max);
    (scaled as u32).min(new_max)
}

#[cfg(test)]
mod tests {
    use marvyr_domain_items::{EquipmentStats, ItemDefinition};
    use marvyr_shared::ids::{ItemDefinitionId, ShipDefinitionId};

    use super::*;

    #[test]
    fn rescale_keeps_the_hull_fraction_and_never_heals_by_cycling() {
        // Casco reforçado num navio inteiro: os +40 entram cheios.
        assert_eq!(rescale_hp(100, 100, 140), 140);
        // Avariado continua na mesma proporção.
        assert_eq!(rescale_hp(50, 100, 140), 70);
        // Tirar o casco: volta à proporção, sem morrer na hora.
        assert_eq!(rescale_hp(70, 140, 100), 50);
        // Equipar e desequipar em ciclo nunca cura.
        let mut hp = 37;
        for _ in 0..20 {
            hp = rescale_hp(rescale_hp(hp, 100, 140), 140, 100);
        }
        assert!(hp <= 37, "{hp}");
        assert_eq!(rescale_hp(5, 0, 100), 5);
    }
    use crate::components::EquippedComponent;
    use crate::definition::ShipKind;
    use marvyr_domain_items::EquipmentSlot;

    fn def() -> ShipDefinition {
        ShipDefinition {
            id: ShipDefinitionId::new(),
            kind: ShipKind::SmallMerchant,
            display_name: String::new(),
            slots: Vec::new(),
            cargo_capacity: 100,
            base_speed: 5.0,
            base_turn_rate: 1.0,
            base_hp: 100,
            base_weapon_damage: 20,
            base_weapon_range: 50.0,
        }
    }

    fn catalog_with(stats: EquipmentStats) -> (ItemCatalog, ItemDefinitionId) {
        let definition = ItemDefinition::equipment(
            ItemDefinitionId::new(),
            String::from("test equipment"),
            10,
            EquipmentSlot::Weapon,
            stats,
        );
        let id = definition.id;
        let mut catalog = ItemCatalog::default();
        catalog.register(definition).unwrap();
        (catalog, id)
    }

    fn component(slot: EquipmentSlot, item_definition: ItemDefinitionId) -> EquippedComponent {
        EquippedComponent {
            slot,
            item_definition,
            affixes: Vec::new(),
        }
    }

    #[test]
    fn affixes_add_points_scale_percentages_and_cap_reload() {
        use marvyr_domain_items::{Affix, AffixKind};
        let (catalog, id) = catalog_with(EquipmentStats::default());
        let affix = |kind, value| Affix { kind, value };
        let mut equipped = EquippedComponents::default();
        equipped.weapon.push(EquippedComponent {
            slot: EquipmentSlot::Weapon,
            item_definition: id,
            affixes: vec![
                affix(AffixKind::Damage, 5),
                affix(AffixKind::Range, 10),
                affix(AffixKind::Reload, 30),
            ],
        });
        equipped.hull.push(EquippedComponent {
            slot: EquipmentSlot::Hull,
            item_definition: id,
            affixes: vec![affix(AffixKind::Hull, 20), affix(AffixKind::Reload, 30)],
        });
        let stats = compute_ship_stats(&def(), &equipped, &catalog).unwrap();
        assert_eq!(stats.weapon_damage, 25);
        assert_eq!(stats.max_hp, 120);
        assert!((stats.weapon_range - 55.0).abs() < 1e-4);
        assert!((stats.reload_factor - 0.6).abs() < 1e-6, "teto de 40%");
    }

    #[test]
    fn gem_costs_slow_the_reload_and_eat_the_hull() {
        use marvyr_domain_items::GemKind;
        let (catalog, id) = catalog_with(EquipmentStats::default());
        let mut equipped = EquippedComponents::default();
        equipped.weapon.push(EquippedComponent {
            slot: EquipmentSlot::Weapon,
            item_definition: id,
            affixes: [GemKind::Ruby, GemKind::Diamond]
                .into_iter()
                .flat_map(GemKind::effects)
                .collect(),
        });
        let stats = compute_ship_stats(&def(), &equipped, &catalog).unwrap();
        assert_eq!(stats.weapon_damage, 28, "Rubi: +8 dano");
        assert!(
            (stats.reload_factor - 1.1).abs() < 1e-6,
            "Rubi: 10% mais lento"
        );
        assert_eq!(stats.max_hp, 90, "Diamante: -10 casco");
        assert!((stats.turn_rate - 1.15).abs() < 1e-6);
    }

    #[test]
    fn no_components_returns_base_stats() {
        let catalog = ItemCatalog::default();
        let stats = compute_ship_stats(&def(), &EquippedComponents::default(), &catalog).unwrap();
        assert_eq!(stats.speed, 5.0);
        assert_eq!(stats.turn_rate, 1.0);
        assert_eq!(stats.max_hp, 100);
        assert_eq!(stats.cargo_capacity, 100);
        assert_eq!(stats.weapon_damage, 20);
        assert_eq!(stats.weapon_range, 50.0);
    }

    #[test]
    fn damage_modifier_adds_to_weapon_damage() {
        let (catalog, id) = catalog_with(EquipmentStats {
            damage: 10,
            ..EquipmentStats::default()
        });
        let equipped = EquippedComponents {
            weapon: vec![component(EquipmentSlot::Weapon, id)],
            ..EquippedComponents::default()
        };
        let stats = compute_ship_stats(&def(), &equipped, &catalog).unwrap();
        assert_eq!(stats.weapon_damage, 30);
    }

    #[test]
    fn damage_modifier_clamps_weapon_damage_at_zero() {
        let (catalog, id) = catalog_with(EquipmentStats {
            damage: -100,
            ..EquipmentStats::default()
        });
        let equipped = EquippedComponents {
            weapon: vec![component(EquipmentSlot::Weapon, id)],
            ..EquippedComponents::default()
        };
        let stats = compute_ship_stats(&def(), &equipped, &catalog).unwrap();
        assert_eq!(stats.weapon_damage, 0);
    }

    #[test]
    fn speed_and_range_modifiers_apply_percent_offsets() {
        let (catalog, id) = catalog_with(EquipmentStats {
            speed: 100,
            range: 100,
            ..EquipmentStats::default()
        });
        let equipped = EquippedComponents {
            sail: vec![component(EquipmentSlot::Sail, id)],
            ..EquippedComponents::default()
        };
        let stats = compute_ship_stats(&def(), &equipped, &catalog).unwrap();
        assert_eq!(stats.speed, 6.0);
        assert_eq!(stats.weapon_range, 51.0);
    }

    #[test]
    fn cargo_and_hp_modifiers_apply() {
        let (catalog, id) = catalog_with(EquipmentStats {
            cargo: 50,
            hp: 25,
            ..EquipmentStats::default()
        });
        let equipped = EquippedComponents {
            hull: vec![component(EquipmentSlot::Hull, id)],
            ..EquippedComponents::default()
        };
        let stats = compute_ship_stats(&def(), &equipped, &catalog).unwrap();
        assert_eq!(stats.cargo_capacity, 150);
        assert_eq!(stats.max_hp, 125);
    }

    #[test]
    fn unknown_equipped_item_fails_closed() {
        let catalog = ItemCatalog::default();
        let equipped = EquippedComponents {
            weapon: vec![component(EquipmentSlot::Weapon, ItemDefinitionId::new())],
            ..EquippedComponents::default()
        };

        assert_eq!(
            compute_ship_stats(&def(), &equipped, &catalog),
            Err(CatalogError::UnknownItem(
                equipped.weapon[0].item_definition
            ))
        );
    }

    #[test]
    fn non_equipment_item_cannot_be_equipped() {
        let id = ItemDefinitionId::new();
        let mut catalog = ItemCatalog::default();
        catalog
            .register(ItemDefinition {
                id,
                kind: marvyr_domain_items::ItemKind::Resource,
                equipment: None,
                max_stack: 10,
                base_weight: 100,
                tags: Default::default(),
                display_name: String::new(),
            })
            .unwrap();
        let equipped = EquippedComponents {
            weapon: vec![component(EquipmentSlot::Weapon, id)],
            ..EquippedComponents::default()
        };

        assert_eq!(
            compute_ship_stats(&def(), &equipped, &catalog),
            Err(CatalogError::NotEquipment(id))
        );
    }
}
