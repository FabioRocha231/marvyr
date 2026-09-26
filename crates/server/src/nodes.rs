//! Nós de recurso no servidor (PRD MF-018/019/020, Phase 6). O node é
//! server-authoritative: estoque, proximidade e porão são julgados aqui. O
//! layout é conteúdo dev — a distribuição regional é o que cria o triângulo
//! econômico (Pilar 2): madeira no Porto da Serra, minério no Porto da Mina,
//! coral raro na ilha sem lei.

use std::time::{Duration, Instant};

use bevy::ecs::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_items::{ItemCatalog, ItemInstance};
use marvyr_domain_world::ResourceNode;
use marvyr_protocol::{GatherNode, GatherResult, NodeState, NodeUpdated, NodesSnapshot};
use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId, ResourceNodeId};
use tracing::{info, warn};

use crate::net::{DevItems, ReliableChannel, ServerGatherPolicy, ServerShip};

/// Node autoritativo no mundo. `node_num` é a face protocolar (u32 estável
/// durante a sessão); `node.id` é a identidade de mundo.
#[derive(Component)]
pub struct ServerNode {
    pub node_num: u32,
    pub node: ResourceNode,
    /// Preenchido quando o estoque zera: quando `Instant::now()` alcançar,
    /// o depósito repovoa (Phase 6: respawn).
    pub respawn_at: Option<Instant>,
}

#[derive(Resource, Default)]
pub struct NodeIdCounter(pub u32);

/// Recurso de cada node: os raros (MF-059) são do próprio depósito; o
/// resto segue a região do slice (MF-020: disponibilidade distinta).
fn resource_of_node(name: &str, region: &str, dev: &DevItems) -> Option<ItemDefinitionId> {
    match name {
        // MV-066: portos livres têm madeira e minério na mesma baía.
        "Mata Costeira" | "Madeira à Deriva" => return Some(dev.timber),
        "Veio Submerso" => return Some(dev.ore),
        "Jazida Costeira" => return Some(dev.ore),
        "Recife Abissal" => return Some(dev.abyssal_pearl),
        "Coração da Cerração" => return Some(dev.fog_essence),
        "Veio Abissal" => return Some(dev.abyssal_amber),
        _ => {}
    }
    match region {
        "Porto da Serra" => Some(dev.timber),
        "Porto da Mina" => Some(dev.ore),
        "Ilha do Coral Negro" => Some(dev.coral),
        _ => None,
    }
}

/// Spawna os nodes dev no mundo (Startup; recursos já inseridos no build).
pub fn spawn_dev_nodes(
    mut commands: Commands,
    map: Res<crate::net::ServerWorldMap>,
    dev: Res<DevItems>,
    policy: Res<ServerGatherPolicy>,
    mut node_ids: ResMut<NodeIdCounter>,
) {
    // MV-065: o layout vem do mapa (clássico ou gerado pela seed).
    let spots = &map.0.features().nodes;
    for spot in spots {
        let region = map
            .0
            .region_by_name(spot.region)
            .unwrap_or_else(|_| panic!("mapa declara a região {}", spot.region));
        let resource = resource_of_node(spot.name, spot.region, &dev)
            .unwrap_or_else(|| panic!("região {} tem recurso dev definido", spot.region));
        let node_num = node_ids.0;
        node_ids.0 += 1;
        commands.spawn((ServerNode {
            node_num,
            node: ResourceNode {
                id: ResourceNodeId::new(),
                name: spot.name,
                x: spot.x,
                y: spot.y,
                region: region.id,
                resource,
                stock: spot.max_stock,
                max_stock: spot.max_stock,
            },
            respawn_at: None,
        },));
    }
    info!(
        nodes = spots.len(),
        radius = policy.0.interact_radius,
        "mundo semeado de recursos"
    );
}

/// Face protocolar de um node: estado + nome do recurso via catálogo.
pub(crate) fn node_state(
    node: &ResourceNode,
    num: u32,
    catalog: &ItemCatalog,
) -> Option<NodeState> {
    let definition = catalog.get(node.resource)?;
    Some(NodeState {
        node_id: num,
        x: node.x,
        y: node.y,
        resource_name: definition.display_name.clone(),
        stock: node.stock,
        max_stock: node.max_stock,
    })
}

/// Mundo inteiro de nodes para o client que acabou de dar hello.
pub fn nodes_snapshot(nodes: &Query<&ServerNode>, catalog: &ItemCatalog) -> NodesSnapshot {
    NodesSnapshot {
        nodes: nodes
            .iter()
            .filter_map(|server_node| node_state(&server_node.node, server_node.node_num, catalog))
            .collect(),
    }
}

/// Coleta (PRD MF-019): perto do node, com estoque e espaço de porão. O
/// servidor corta o pedido ao que couber — nada se perde no mar.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
pub fn handle_gather(
    mut gather_events: EventReader<ServerReceiveMessage<GatherNode>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    policy: Res<ServerGatherPolicy>,
    mut metrics: ResMut<crate::net::Metrics>,
    mut ships: Query<&mut ServerShip>,
    mut nodes: Query<&mut ServerNode>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
    talents: Res<crate::talents::CaptainTalents>,
) {
    for event in gather_events.read() {
        let client_id = event.from();
        let node_num = event.message().node_id;

        let Some(mut ship) = ships
            .iter_mut()
            .find(|ship| ship.client_id == Some(client_id))
        else {
            continue;
        };
        // MF-036: coleta é ação de mar — atracado, o casco não pega machado.
        if matches!(
            ship.presence,
            marvyr_domain_ships::VesselPresence::Docked(_)
        ) {
            info!(node_num, "coleta recusada: atracado (MF-036)");
            send_failure(
                &mut connection_manager,
                client_id,
                node_num,
                "Atracado: desatraque para coletar no mar.",
            );
            continue;
        };
        let Some(mut server_node) = nodes
            .iter_mut()
            .find(|server_node| server_node.node_num == node_num)
        else {
            warn!(node_num, "coleta de nó inexistente");
            send_failure(
                &mut connection_manager,
                client_id,
                node_num,
                "Recurso não encontrado: procure outro no mapa.",
            );
            continue;
        };

        if !server_node
            .node
            .in_range(ship.motion.x, ship.motion.y, policy.0.interact_radius)
        {
            info!(node_num, "longe demais do nó para coletar");
            send_failure(
                &mut connection_manager,
                client_id,
                node_num,
                "Longe demais do recurso: chegue mais perto.",
            );
            continue;
        }
        if server_node.node.is_depleted() {
            info!(node_num, "nó esgotado; aguarde o respawn");
            send_failure(
                &mut connection_manager,
                client_id,
                node_num,
                "Recurso esgotado: volte mais tarde.",
            );
            continue;
        }

        // Quanto cabe no porão? (fail-closed pelo catálogo — ADR-0006)
        let Some(definition) = dev.catalog.get(server_node.node.resource) else {
            warn!(node_num, "recurso do nó fora do catálogo; recusado");
            send_failure(
                &mut connection_manager,
                client_id,
                node_num,
                "Recurso indisponível: tente outro.",
            );
            continue;
        };
        let free = ship
            .hold
            .free_weight(&dev.catalog)
            .expect("porão só contém definições do catálogo");
        let affordable = free / definition.base_weight.max(1);
        let amount = policy
            .0
            .amount_per_gather
            .min(server_node.node.stock)
            .min(affordable);
        if amount == 0 {
            info!(node_num, "porão cheio: coleta rejeitada");
            send_failure(
                &mut connection_manager,
                client_id,
                node_num,
                "Porão cheio: venda ou guarde carga no porto.",
            );
            continue;
        }

        let taken = server_node.node.take(amount);
        // Rosa dos Ventos: coletor treinado tira um pouco a mais (cabe no porão).
        let extra = talents
            .bonus(ship.character)
            .gather_extra(taken)
            .min(affordable - taken);
        ship.hold
            .insert(
                &dev.catalog,
                ItemInstance::new_resource(
                    ItemInstanceId::new(),
                    server_node.node.resource,
                    taken + extra,
                ),
            )
            .expect("cabe: o espaço foi conferido acima");
        metrics.items_gathered += u64::from(taken + extra);
        renown.send(crate::renown::RenownEarned {
            character: ship.character,
            amount: taken * marvyr_domain_economy::renown::PER_GATHERED_UNIT,
            reason: "coleta",
        });
        if server_node.node.is_depleted() {
            server_node.respawn_at =
                Some(Instant::now() + Duration::from_secs_f32(policy.0.respawn_secs));
        }

        if let Some(state) = node_state(&server_node.node, node_num, &dev.catalog) {
            let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
                &NodeUpdated { node: state },
                NetworkTarget::All,
            );
        }
        let _ = connection_manager.send_message::<ReliableChannel, _>(
            client_id,
            &GatherResult {
                node_id: node_num,
                success: true,
                gathered: taken + extra,
                reason: String::new(),
            },
        );
        // MV-061: às vezes a rede traz um mapa do tesouro junto.
        if let Some(quality) = crate::seafaring::maybe_find_map(&mut ship, &dev) {
            // v26: mapa Mágico/Raro já avisa o perigo (e o tamanho do baú).
            let text = match quality {
                Some(quality) => format!(
                    "Um Mapa do Tesouro {} veio na rede! {} · +{}% no baú.",
                    if quality.rarity == marvyr_domain_items::Rarity::Rare {
                        "Raro"
                    } else {
                        "Mágico"
                    },
                    quality
                        .map_mods
                        .iter()
                        .map(|m| m.label())
                        .collect::<Vec<_>>()
                        .join(", "),
                    marvyr_domain_items::map_mod::treasure_bonus_pct(&quality.map_mods),
                ),
                None => String::from("Um Mapa do Tesouro veio na rede! Veja o X na carta."),
            };
            crate::reputation::send_event(
                &mut connection_manager,
                &[client_id],
                text,
                marvyr_protocol::WorldEventKind::Bounty,
            );
        }
        info!(
            ship_id = ship.ship_id,
            node_num,
            gathered = taken + extra,
            resource = %definition.display_name,
            "recursos coletados"
        );
    }
}

fn send_failure(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    node_id: u32,
    reason: &str,
) {
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &GatherResult {
            node_id,
            success: false,
            gathered: 0,
            reason: reason.to_owned(),
        },
    );
}

/// Respawn (Phase 6): depósito esgotado repovoa após a política.
pub fn respawn_nodes(
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    mut nodes: Query<&mut ServerNode>,
) {
    let now = Instant::now();
    for mut server_node in &mut nodes {
        let Some(when) = server_node.respawn_at else {
            continue;
        };
        if now < when {
            continue;
        }
        server_node.node.refill();
        server_node.respawn_at = None;
        if let Some(state) = node_state(&server_node.node, server_node.node_num, &dev.catalog) {
            let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
                &NodeUpdated { node: state },
                NetworkTarget::All,
            );
        }
        info!(node_num = server_node.node_num, "nó repovoado");
    }
}

#[cfg(test)]
mod tests {
    use marvyr_domain_world::{NodeSpot, WorldMap};

    fn classic_nodes() -> Vec<NodeSpot> {
        WorldMap::vertical_slice().features().nodes.clone()
    }

    /// MF-058: terra é obstáculo — node, spawn de jogador e spawn de NPC
    /// precisam estar na água, com folga de casco.
    #[test]
    fn nodes_and_spawns_are_on_open_water() {
        let map = WorldMap::vertical_slice();
        let clearance = crate::net::HULL_CLEARANCE;
        for spot in classic_nodes() {
            assert!(
                map.push_out_of_land(spot.x, spot.y, clearance).is_none(),
                "{} ({},{})",
                spot.name,
                spot.x,
                spot.y
            );
        }
        let spawn = map.features().spawn;
        assert!(map.push_out_of_land(spawn.0, spawn.1, clearance).is_none());
        for (x, y) in crate::npc::NpcSpawnConfig::default().spawn_positions {
            assert!(
                map.push_out_of_land(x, y, clearance).is_none(),
                "NPC ({x},{y})"
            );
        }
    }

    /// A distribuição regional do slice tem que ser distinta (MF-020) e a
    /// geografia honesta: cada node dentro das águas da sua região, e o
    /// recurso raro só na ilha sem lei.
    #[test]
    fn layout_matches_triangular_economy() {
        let map = WorldMap::vertical_slice();
        let nodes = classic_nodes();
        let count = |region: &str| nodes.iter().filter(|spot| spot.region == region).count();
        assert_eq!(count("Porto da Serra"), 5);
        assert_eq!(count("Porto da Mina"), 5);
        // Coral na ilha + raros das zonas de alto risco (MF-059).
        assert_eq!(count("Ilha do Coral Negro"), 16);

        // Cada node fica dentro de uma zona declarada da sua região —
        // madeira/minério em águas protegidas, coral em lawless.
        for NodeSpot {
            name,
            region,
            x,
            y,
            max_stock,
        } in nodes
        {
            let zone = map
                .zone_at(x, y)
                .unwrap_or_else(|_| panic!("node {name} fora do mar declarado"));
            let expected_tier = if region == "Ilha do Coral Negro" {
                marvyr_domain_world::RiskTier::Lawless
            } else {
                marvyr_domain_world::RiskTier::Protected
            };
            assert_eq!(
                zone.tier, expected_tier,
                "node {name} ({x}, {y}) na zona errada"
            );
            assert!(max_stock > 0);
        }
    }

    /// O node da saída da baía fica no caminho do spawn para leste — o dev
    /// smoke (AUTOSAIL) cruza em faixa de coleta sem manobra.
    #[test]
    fn route_node_is_on_the_dev_sail_path() {
        let map = WorldMap::vertical_slice();
        let road = classic_nodes()
            .into_iter()
            .find(|spot| spot.name == "Bosque do Caminho")
            .expect("node do caminho existe");
        let (spawn_x, spawn_y) = map.features().spawn;
        assert!((road.x - spawn_x).abs() < 200.0, "node perto da doca");
        assert!(
            (road.y - spawn_y).abs() < 30.0,
            "node na linha de navegação"
        );
    }
}
