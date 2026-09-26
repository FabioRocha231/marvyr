//! Crafting no servidor (PRD §36-§38, MF-021/022). Catálogos dev (receitas
//! de equipamento e ordens de construção de navio), disponibilidade de
//! estações por área de porto (§5: portos são áreas de serviço) e o handler
//! do intent `CraftItem` — fail-closed ponta a ponta (§37).

use bevy::ecs::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_combat::FlaskKind;
use marvyr_domain_crafting::{
    can_construct, CraftError, Ingredient, Recipe, ShipConstructionJob, StationKind,
};
use marvyr_domain_items::{GemKind, ItemCatalog, OrbKind, Quality, Rarity};
use marvyr_domain_ships::{ShipDefinition, ShipKind, VesselPresence};
use marvyr_domain_world::map::PIRATE_PORT;
use marvyr_domain_world::WorldMap;
use marvyr_protocol::{AssignShip, CraftItem, CraftResult, RecipeEntry, RecipesSnapshot};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId, RecipeId, RegionId};
use tracing::{info, warn};

use crate::net::{spawn_ship_for, DevItems, ReliableChannel, ServerShip, ServerWorldMap};

/// Registro dev de definições de navio (MF-022): os três cascos do §11.
/// Sem `Default` de propósito: cada instância carrega ids próprios, e um
/// default implícito esconderia isso (mesma razão do `new_without_default`).
#[allow(clippy::new_without_default)]
#[derive(Resource)]
pub struct DevShips {
    pub merchant: ShipDefinition,
    pub patrol: ShipDefinition,
    pub corsair: ShipDefinition,
}

impl DevShips {
    /// Sem `Default` de propósito (mesma razão do allow): cada instância
    /// carrega ids próprios de definição, e um default implícito esconderia
    /// a geração.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            merchant: ShipDefinition::small_merchant(),
            patrol: ShipDefinition::patrol(),
            corsair: ShipDefinition::corsair(),
        }
    }

    pub fn definition(&self, kind: ShipKind) -> &ShipDefinition {
        match kind {
            ShipKind::SmallMerchant => &self.merchant,
            ShipKind::Patrol => &self.patrol,
            ShipKind::Corsair => &self.corsair,
        }
    }
}

/// Catálogo dev de receitas (PRD §39: valores são balanceamento).
#[derive(Resource)]
pub struct DevRecipes {
    pub equipment: Vec<Recipe>,
    pub ships: Vec<ShipConstructionJob>,
}

impl DevRecipes {
    pub fn new(dev: &DevItems) -> Self {
        let ingredient = |item: ItemDefinitionId, quantity: u32| Ingredient { item, quantity };
        let equipment_recipe =
            |display_name: &str, output: ItemDefinitionId, ingredients: Vec<Ingredient>| Recipe {
                id: RecipeId::new(),
                display_name: String::from(display_name),
                output_item: output,
                output_quantity: 1,
                ingredients,
                required_station: StationKind::Workbench,
                craft_time_secs: 0,
                output_rarity: Default::default(),
                output_tier: 1,
            };

        let equipment = vec![
            equipment_recipe(
                "Casco Reforçado",
                dev.hull_plate,
                vec![ingredient(dev.timber, 15)],
            ),
            equipment_recipe(
                "Velas de Corrida",
                dev.racing_sails,
                vec![ingredient(dev.timber, 10), ingredient(dev.ore, 10)],
            ),
            equipment_recipe(
                "Canhão de Bronze",
                dev.bronze_cannon,
                vec![ingredient(dev.ore, 20), ingredient(dev.coral, 5)],
            ),
            // MF-059: tier 2 só na Forja Pirata (Anvil do Porto do Coral
            // Negro) — o recurso raro tem que atravessar as Águas Negras.
            Recipe {
                required_station: StationKind::Anvil,
                output_tier: 2,
                ..equipment_recipe(
                    "Casco Negro",
                    dev.black_hull,
                    vec![ingredient(dev.timber, 20), ingredient(dev.abyssal_pearl, 6)],
                )
            },
            Recipe {
                required_station: StationKind::Anvil,
                output_tier: 2,
                ..equipment_recipe(
                    "Velas de Cerração",
                    dev.fog_sails,
                    vec![
                        ingredient(dev.timber, 10),
                        ingredient(dev.fog_essence, 5),
                        ingredient(dev.coral, 5),
                    ],
                )
            },
            Recipe {
                required_station: StationKind::Anvil,
                output_tier: 2,
                ..equipment_recipe(
                    "Canhões Abissais",
                    dev.abyssal_cannons,
                    vec![ingredient(dev.ore, 20), ingredient(dev.abyssal_amber, 5)],
                )
            },
            // MV-066: tier 3 — cristal dos baús da cerração sobre o tier 2.
            Recipe {
                required_station: StationKind::Anvil,
                output_tier: 3,
                ..equipment_recipe(
                    "Casco de Cristal",
                    dev.crystal_hull,
                    vec![
                        ingredient(dev.timber, 25),
                        ingredient(dev.abyssal_pearl, 4),
                        ingredient(dev.fog_crystal, 4),
                    ],
                )
            },
            Recipe {
                required_station: StationKind::Anvil,
                output_tier: 3,
                ..equipment_recipe(
                    "Canhões de Cristal",
                    dev.crystal_cannons,
                    vec![
                        ingredient(dev.ore, 25),
                        ingredient(dev.abyssal_amber, 4),
                        ingredient(dev.fog_crystal, 4),
                    ],
                )
            },
        ];
        // v24: gemas de suporte na oficina do Porto da Serra. Cada uma pede
        // o recurso de uma rota (coral da ilha sem lei, raros das zonas de
        // risco): gema boa vem de quem navega.
        let mut equipment = equipment;
        for (gem, ingredients) in [
            (
                GemKind::Ruby,
                vec![ingredient(dev.ore, 8), ingredient(dev.coral, 3)],
            ),
            (
                GemKind::Sapphire,
                vec![ingredient(dev.timber, 8), ingredient(dev.coral, 3)],
            ),
            (
                GemKind::Emerald,
                vec![
                    ingredient(dev.timber, 6),
                    ingredient(dev.ore, 6),
                    ingredient(dev.coral, 2),
                ],
            ),
            (
                GemKind::Topaz,
                vec![ingredient(dev.ore, 12), ingredient(dev.abyssal_amber, 1)],
            ),
            (
                GemKind::Amethyst,
                vec![ingredient(dev.timber, 12), ingredient(dev.fog_essence, 1)],
            ),
            (
                GemKind::Diamond,
                vec![ingredient(dev.ore, 10), ingredient(dev.abyssal_pearl, 1)],
            ),
        ] {
            equipment.push(equipment_recipe(
                gem.item_name(),
                gem.item_id(),
                ingredients,
            ));
        }
        // v29: orbes de ofício. O caro é o recurso das rotas de risco: a
        // pedra que mexe no Raro vem das águas sem lei e da cerração.
        for (orb, ingredients) in [
            (
                OrbKind::Transmutation,
                vec![ingredient(dev.ore, 6), ingredient(dev.coral, 2)],
            ),
            (
                OrbKind::Chaos,
                vec![ingredient(dev.coral, 3), ingredient(dev.abyssal_pearl, 1)],
            ),
            (
                OrbKind::Regal,
                vec![ingredient(dev.coral, 2), ingredient(dev.fog_crystal, 1)],
            ),
            (
                OrbKind::Exalted,
                vec![
                    ingredient(dev.abyssal_amber, 1),
                    ingredient(dev.abyssal_pearl, 1),
                ],
            ),
            (
                OrbKind::Cartographer,
                vec![ingredient(dev.timber, 5), ingredient(dev.fog_essence, 1)],
            ),
        ]
        .into_iter()
        // v33: Selos dos aspectos lendários — cinza da Maré Sangrenta,
        // cristal da Cerração e âmbar do sem-lei: o topo do risco.
        .chain(marvyr_domain_items::AspectKind::ALL.map(|aspect| {
            (
                OrbKind::Seal(aspect),
                vec![
                    ingredient(dev.blood_ash, 10),
                    ingredient(dev.fog_crystal, 1),
                    ingredient(dev.abyssal_amber, 2),
                ],
            )
        })) {
            equipment.push(equipment_recipe(
                orb.item_name(),
                orb.item_id(),
                ingredients,
            ));
        }
        // v25: frascos de bordo, baratos de propósito — o que custa é
        // carregá-los: vão no porão e afundam junto.
        for (kind, ingredients) in [
            (FlaskKind::Repair, vec![ingredient(dev.timber, 12)]),
            (
                FlaskKind::Wind,
                vec![ingredient(dev.timber, 8), ingredient(dev.ore, 4)],
            ),
            (
                FlaskKind::Fury,
                vec![ingredient(dev.ore, 10), ingredient(dev.coral, 2)],
            ),
            (
                FlaskKind::Tar,
                vec![
                    ingredient(dev.timber, 6),
                    ingredient(dev.ore, 6),
                    ingredient(dev.coral, 1),
                ],
            ),
        ] {
            equipment.push(equipment_recipe(
                kind.item_name(),
                kind.item_id(),
                ingredients,
            ));
        }

        let ships = vec![
            ShipConstructionJob {
                id: RecipeId::new(),
                display_name: String::from("Patrol"),
                kind: ShipKind::Patrol,
                ingredients: vec![ingredient(dev.timber, 30), ingredient(dev.ore, 30)],
                required_station: StationKind::Dock,
            },
            ShipConstructionJob {
                id: RecipeId::new(),
                display_name: String::from("Corsair"),
                kind: ShipKind::Corsair,
                ingredients: vec![ingredient(dev.ore, 40), ingredient(dev.coral, 10)],
                required_station: StationKind::Dock,
            },
        ];

        Self { equipment, ships }
    }

    /// Receita por número de protocolo: equipamento primeiro, navios depois.
    pub fn equipment_for(&self, num: u32) -> Option<&Recipe> {
        self.equipment.get(num as usize)
    }

    pub fn ship_for(&self, num: u32) -> Option<&ShipConstructionJob> {
        let offset = self.equipment.len() as u32;
        num.checked_sub(offset)
            .and_then(|index| self.ships.get(index as usize))
    }

    /// Catálogo completo para o client no handshake (MF-021/022).
    pub fn snapshot(&self, catalog: &ItemCatalog, catalyst: ItemDefinitionId) -> RecipesSnapshot {
        let lines = |ingredients: &[Ingredient]| {
            ingredients
                .iter()
                .map(|ingredient| marvyr_protocol::IngredientLine {
                    name: catalog
                        .get(ingredient.item)
                        .map(|definition| definition.display_name.clone())
                        .unwrap_or_default(),
                    quantity: ingredient.quantity,
                })
                .collect()
        };
        // Só equipamento tem raridade (afixo não existe em recurso).
        let rarity_lines = |recipe: &Recipe, rarity: Rarity| {
            if catalog
                .get(recipe.output_item)
                .is_some_and(|definition| definition.is_equipment())
            {
                lines(&recipe.at_rarity(rarity, catalyst).ingredients)
            } else {
                Vec::new()
            }
        };
        let mut recipes: Vec<RecipeEntry> = self
            .equipment
            .iter()
            .enumerate()
            .map(|(num, recipe)| RecipeEntry {
                recipe_id: num as u32,
                display_name: recipe.display_name.clone(),
                station: recipe.required_station,
                ship_build: false,
                output_name: catalog
                    .get(recipe.output_item)
                    .map(|definition| definition.display_name.clone())
                    .unwrap_or_default(),
                output_quantity: recipe.output_quantity,
                ingredients: lines(&recipe.ingredients),
                magic: rarity_lines(recipe, Rarity::Magic),
                rare: rarity_lines(recipe, Rarity::Rare),
            })
            .collect();
        let offset = self.equipment.len() as u32;
        for (index, job) in self.ships.iter().enumerate() {
            recipes.push(RecipeEntry {
                recipe_id: offset + index as u32,
                display_name: job.display_name.clone(),
                station: job.required_station,
                ship_build: true,
                output_name: job.display_name.clone(),
                output_quantity: 1,
                ingredients: lines(&job.ingredients),
                magic: Vec::new(),
                rare: Vec::new(),
            });
        }
        RecipesSnapshot { recipes }
    }
}

/// Disponibilidade de estação no porto da região (PRD §5/§7, MF-036/037):
/// a oficina é do porto ONDE ESTÁ ATRACADO — água protegida não é doca. O
/// Porto da Serra tem Workbench + Dock; o Porto da Mina, Dock. Anvil não
/// é a Forja Pirata do Porto do Coral Negro (MF-059); região sem porto não
/// tem estação alguma.
pub fn station_available(map: &WorldMap, region: RegionId, required: StationKind) -> bool {
    match required {
        StationKind::None => true,
        StationKind::Workbench | StationKind::Anvil | StationKind::Dock => {
            let Some(known) = map
                .regions()
                .iter()
                .find(|candidate| candidate.id == region)
            else {
                return false;
            };
            let port_name = known.port.as_ref().map(|port| port.name);
            match required {
                StationKind::Workbench => known.name == "Porto da Serra",
                // Forja Pirata: só no porto das Águas Negras (MF-059).
                StationKind::Anvil => port_name == Some(PIRATE_PORT),
                _ => port_name.is_some(), // Dock: toda região com porto
            }
        }
    }
}

/// Estação "efetiva" para a regra pura: a exigida quando disponível, `None`
/// quando não — `can_craft` responde `WrongStation` (fail-closed).
fn effective_station(map: &WorldMap, region: RegionId, required: StationKind) -> StationKind {
    if station_available(map, region, required) {
        required
    } else {
        StationKind::None
    }
}

/// Intent de fabricação/construção (PRD §63, MF-036/037). A oficina é
/// serviço de porto: só com o navio ATRACADO, e os insumos vêm do STORAGE
/// regional — o porão não é matéria-prima automática (Pilar 2).
#[allow(clippy::too_many_arguments)]
pub fn handle_craft(
    mut commands: Commands,
    mut craft_events: EventReader<ServerReceiveMessage<CraftItem>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    dev_ships: Res<DevShips>,
    dev_recipes: Res<DevRecipes>,
    mut metrics: ResMut<crate::net::Metrics>,
    map: Res<ServerWorldMap>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut ship_ids: ResMut<crate::net::ShipIdCounter>,
    mut ships: Query<(Entity, &mut ServerShip)>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
) {
    for event in craft_events.read() {
        let client_id = event.from();
        let recipe_num = event.message().recipe_id;
        let rarity = event.message().rarity;
        let Some((ship_entity, mut ship)) = ships
            .iter_mut()
            .find(|(_, ship)| ship.client_id == Some(client_id))
        else {
            continue;
        };

        // MF-036: sem doca, sem oficina.
        let VesselPresence::Docked(region) = ship.presence else {
            info!(
                ship_id = ship.ship_id,
                "craft recusado: atraca primeiro (E) — oficina é serviço de porto"
            );
            send_craft_result(
                &mut connection_manager,
                client_id,
                recipe_num,
                Err(String::from(
                    "Precisa estar atracado num porto com oficina: atraque com E.",
                )),
            );
            continue;
        };
        let character = ship.character;

        // 1. Equipamento (MF-021/037): insumos do storage → item no storage.
        if let Some(recipe) = dev_recipes.equipment_for(recipe_num) {
            let station = effective_station(&map.0, region, recipe.required_station);
            // Afixo: a versão Mágica/Rara custa mais e o Raro pede Coral Negro.
            let is_equipment = dev
                .catalog
                .get(recipe.output_item)
                .is_some_and(|definition| definition.is_equipment());
            if rarity != Rarity::Normal && !is_equipment {
                send_craft_result(
                    &mut connection_manager,
                    client_id,
                    recipe_num,
                    Err(String::from("Só equipamento sai Mágico ou Raro.")),
                );
                continue;
            }
            let recipe = &recipe.at_rarity(rarity, dev.coral);
            match market.craft_at_storage(character, region, recipe, &dev.catalog, station) {
                Ok(output) => {
                    metrics.items_crafted += u64::from(output.quantity.max(1));
                    let name = dev
                        .catalog
                        .get(output.definition)
                        .map(|definition| definition.display_name.clone())
                        .unwrap_or_default();
                    info!(
                        ship_id = ship.ship_id,
                        recipe = %recipe.display_name,
                        output = %name,
                        "equipamento fabricado no storage do porto"
                    );
                    renown.send(crate::renown::RenownEarned {
                        character,
                        amount: marvyr_domain_economy::renown::PER_CRAFT,
                        reason: "fabricação",
                    });
                    send_craft_result(
                        &mut connection_manager,
                        client_id,
                        recipe_num,
                        Ok(output.quality),
                    );
                }
                Err(error) => {
                    warn!(
                        ship_id = ship.ship_id,
                        recipe = %recipe.display_name,
                        error = %error,
                        "craft recusado (insumo no storage? deposite com Z)"
                    );
                    send_craft_result(
                        &mut connection_manager,
                        client_id,
                        recipe_num,
                        Err(craft_error_reason(&error, &dev.catalog)),
                    );
                }
            }
            continue;
        }

        // 2. Navio (MF-022/037): Dock + insumos do storage → ShipInstance nova.
        if let Some(job) = dev_recipes.ship_for(recipe_num) {
            let built = build_ship_for_job(
                &mut commands,
                &mut connection_manager,
                &mut metrics,
                &dev,
                &dev_ships,
                &map.0,
                &mut market,
                &mut ship_ids,
                job,
                ship_entity,
                &mut ship,
                region,
            );
            if built.is_ok() {
                // Casco novo vale como quatro fabricações.
                renown.send(crate::renown::RenownEarned {
                    character,
                    amount: 4 * marvyr_domain_economy::renown::PER_CRAFT,
                    reason: "navio construído",
                });
            }
            send_craft_result(
                &mut connection_manager,
                client_id,
                recipe_num,
                built.map(|()| None),
            );
            continue;
        }

        warn!(recipe_num, "receita desconhecida (fail-closed)");
        send_craft_result(
            &mut connection_manager,
            client_id,
            recipe_num,
            Err(String::from(
                "Receita desconhecida: reabra a oficina e escolha outra.",
            )),
        );
    }
}

/// Valida e executa a construção (MF-022/037): insumos vêm do STORAGE da
/// região, a carga embarcada do casco antigo migra para o novo (§38: ShipInstance
/// é entidade própria — a carga acompanha o dono, não some), e o casco velho
/// é aposentado. Fail-closed: validação antes de consumo, consumo antes de spawn.
#[allow(clippy::too_many_arguments)]
fn build_ship_for_job(
    commands: &mut Commands,
    connection_manager: &mut ConnectionManager,
    metrics: &mut crate::net::Metrics,
    dev: &DevItems,
    dev_ships: &DevShips,
    map: &WorldMap,
    market: &mut crate::market::ServerMarket,
    ship_ids: &mut crate::net::ShipIdCounter,
    job: &ShipConstructionJob,
    old_entity: Entity,
    old_ship: &mut ServerShip,
    region: RegionId,
) -> Result<(), String> {
    let character = old_ship.character;
    let station = effective_station(map, region, job.required_station);
    // Insumos contam contra o STORAGE (MF-037) — não contra o porão.
    let mut quantities = std::collections::HashMap::new();
    for ingredient in &job.ingredients {
        let have = market.storage_quantity(character, region, ingredient.item);
        quantities.insert(ingredient.item, have);
    }
    if let Err(error) = can_construct(job, &quantities, station) {
        warn!(
            ship_id = old_ship.ship_id,
            job = %job.display_name,
            error = %error,
            "construção recusada (insumos no storage? deposite com Z)"
        );
        return Err(craft_error_reason(&error, &dev.catalog));
    }
    // A carga atual precisa caber no casco novo (§38) — ela migra inteira.
    let used = old_ship
        .hold
        .used_weight(&dev.catalog)
        .expect("porão só contém definições do catálogo");
    let new_definition = dev_ships.definition(job.kind);
    if used > new_definition.cargo_capacity {
        warn!(
            ship_id = old_ship.ship_id,
            job = %job.display_name,
            used,
            capacity = new_definition.cargo_capacity,
            "carga não cabe no casco novo; descarregue antes"
        );
        return Err(format!(
            "Carga não cabe no casco novo ({used}/{}): guarde carga no porto antes.",
            new_definition.cargo_capacity
        ));
    }

    let cargo: Vec<_> = old_ship.hold.items().to_vec();
    // Consome os insumos do storage (validado acima).
    for ingredient in &job.ingredients {
        if let Err(error) =
            market.consume_from_storage(character, region, ingredient.item, ingredient.quantity)
        {
            warn!(error = %error, "consumo do storage falhou após validação; construção abortada");
            return Err(String::from(
                "Materiais mudaram no armazém: confira e tente de novo.",
            ));
        }
    }

    let owner_client = old_ship.client_id;
    let owner_character = old_ship.character;
    let old_ship_id = old_ship.ship_id;
    // O equipamento instalado não vai para o casco novo nem some com o
    // velho: volta ao armazém deste porto (cada item mora em um lugar).
    retire_equipment(
        market,
        &mut old_ship.loadout,
        character,
        region,
        &dev.catalog,
    );
    commands.entity(old_entity).despawn();
    metrics.ships_constructed += 1;
    let new_ship_id = spawn_ship_for(
        commands,
        ship_ids,
        dev,
        dev_ships,
        map,
        job.kind,
        owner_client,
        owner_character,
        cargo,
        Some(region),
    );
    if let Some(client_id) = owner_client {
        let _ = connection_manager.send_message::<ReliableChannel, _>(
            client_id,
            &AssignShip {
                ship_id: new_ship_id,
                kind: job.kind,
            },
        );
        // O casco novo nasce atracado aqui, no porto da obra — não na doca
        // inicial (o jogador era teleportado para longe do próprio armazém).
        let berth = crate::net::restored_position(
            map,
            marvyr_domain_ships::VesselPresence::Docked(region),
            0.0,
            0.0,
        );
        if let Some(zone) = crate::net::zone_changed_for(map, new_ship_id, berth.0, berth.1) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(client_id, &zone);
        }
        crate::loadout::send_loadout_snapshot(
            connection_manager,
            client_id,
            new_definition,
            &dev.catalog,
            &[],
        );
    }
    info!(
        old_ship_id,
        new_ship_id,
        job = %job.display_name,
        ?station,
        "navio construído no Dock com insumos do storage"
    );
    Ok(())
}

/// Desinstala tudo do casco que sai de cena e devolve ao armazém do porto.
fn retire_equipment(
    market: &mut crate::market::ServerMarket,
    loadout: &mut marvyr_domain_ships::ShipLoadout,
    character: CharacterId,
    region: RegionId,
    catalog: &ItemCatalog,
) {
    let slots: Vec<_> = loadout
        .items()
        .filter_map(|custody| match custody.location {
            marvyr_domain_items::ItemLocation::Equipped { slot, .. } => Some(slot),
            _ => None,
        })
        .collect();
    for slot in slots {
        if let Some(custody) = loadout.unequip(slot) {
            market.return_to_storage(character, region, custody, catalog);
        }
    }
}

/// Motivo curto e acionável (PT-BR) para o jogador a partir do erro de craft.
fn craft_error_reason(error: &CraftError, catalog: &ItemCatalog) -> String {
    let name = |item| {
        catalog
            .get(item)
            .map(|definition| definition.display_name.clone())
            .unwrap_or_else(|| String::from("material"))
    };
    match error {
        CraftError::MissingIngredient {
            item,
            needed,
            available,
        } => format!(
            "Faltam materiais: {} {} ({available}/{needed} no armazém). Deposite com Z.",
            needed - available,
            name(*item)
        ),
        CraftError::WrongStation { required, .. } => match required {
            StationKind::Workbench => {
                String::from("Precisa da oficina: atraque no Porto da Serra.")
            }
            StationKind::Anvil => {
                String::from("Precisa da Forja Pirata: atraque no porto das Águas Negras.")
            }
            _ => String::from("Precisa estar atracado num porto com estaleiro."),
        },
        CraftError::EmptyStorage => {
            String::from("Armazém vazio neste porto: deposite materiais com Z.")
        }
        CraftError::NoRoomForOutput | CraftError::Cargo(_) => {
            String::from("Sem espaço para o item: libere espaço no armazém.")
        }
        CraftError::UnknownOutputItem { .. } => {
            String::from("Receita indisponível: escolha outra.")
        }
    }
}

fn send_craft_result(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    recipe_id: u32,
    outcome: Result<Option<Quality>, String>,
) {
    let (success, reason, quality) = match outcome {
        Ok(quality) => (true, String::new(), quality),
        Err(reason) => (false, reason, None),
    };
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &CraftResult {
            recipe_id,
            success,
            reason,
            quality,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trocar de navio não some com o equipamento: tudo que estava
    /// instalado volta ao armazém do porto da obra, uma vez só.
    #[test]
    fn retired_hull_returns_equipment_to_port_storage() {
        use marvyr_domain_items::{Custody, EquipmentSlot, ItemInstance, ItemLocation};
        use marvyr_shared::ids::{ItemInstanceId, ShipInstanceId};

        let dev = DevItems::new();
        let mut market = crate::market::ServerMarket::new();
        let character = CharacterId::new();
        let region = RegionId::new();
        let ship = ShipInstanceId::new();
        let mut loadout = marvyr_domain_ships::ShipLoadout::new();
        for (definition, slot) in [
            (dev.hull_plate, EquipmentSlot::Hull),
            (dev.bronze_cannon, EquipmentSlot::Weapon),
        ] {
            let instance = ItemInstance::new_equipment(ItemInstanceId::new(), definition, 100);
            let custody = Custody {
                instance,
                location: ItemLocation::ShipCargo(ship),
            };
            loadout.equip(ship, custody, slot);
        }

        retire_equipment(&mut market, &mut loadout, character, region, &dev.catalog);

        assert_eq!(loadout.items().count(), 0, "casco velho fica vazio");
        assert_eq!(
            market.storage_quantity(character, region, dev.hull_plate),
            1
        );
        assert_eq!(
            market.storage_quantity(character, region, dev.bronze_cannon),
            1
        );
    }

    /// §7: Serra tem Workbench + Dock; Mina, só Dock; mar aberto, nada;
    /// Anvil não existe no slice.
    #[test]
    fn stations_follow_port_specialization_by_region() {
        let map = WorldMap::vertical_slice();
        let serra = map.region_by_name("Porto da Serra").unwrap().id;
        let mina = map.region_by_name("Porto da Mina").unwrap().id;
        let ilha = map.region_by_name("Ilha do Coral Negro").unwrap().id;

        // Serra: Workbench + Dock (especialização de porto, §7).
        assert!(station_available(&map, serra, StationKind::Workbench));
        assert!(station_available(&map, serra, StationKind::Dock));
        // Mina: só Dock.
        assert!(!station_available(&map, mina, StationKind::Workbench));
        assert!(station_available(&map, mina, StationKind::Dock));
        // Ilha (MF-059): porto pirata com Dock e Forja (Anvil), sem Workbench;
        // a Forja não existe nos portos da coroa.
        assert!(station_available(&map, ilha, StationKind::Dock));
        assert!(station_available(&map, ilha, StationKind::Anvil));
        assert!(!station_available(&map, ilha, StationKind::Workbench));
        assert!(!station_available(&map, serra, StationKind::Anvil));
        assert!(!station_available(&map, mina, StationKind::Anvil));
        assert!(station_available(&map, serra, StationKind::None));
    }
}
