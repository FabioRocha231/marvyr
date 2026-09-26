//! Guilda Mercante e Quadro de Contratos no client: abas "Guilda" e
//! "Contratos" da tela de porto (corpo montado aqui) e a linha do contrato
//! ativo no HUD do mar, logo abaixo do painel do navio. Tudo é leitura dos
//! snapshots do servidor; os botões só emitem intents (Pilar 4).

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_protocol::{
    AbandonContract, AcceptContract, ContractLine, ContractResult, ContractsSnapshot, GuildPrices,
    SellToGuild, StorageDepositAll, StorageLine, Undock,
};
use marvyr_shared::ids::ItemDefinitionId;

use crate::hud::SeaHud;
use crate::i18n::{tr, trf};
use crate::net::ReliableChannel;
use crate::ui;

#[derive(Resource, Debug, Default)]
pub struct KnownGuildPrices(pub Option<GuildPrices>);

/// Último snapshot de contratos + instante de chegada (contagem local).
#[derive(Resource, Debug, Default)]
pub struct KnownContracts {
    pub snapshot: Option<ContractsSnapshot>,
    pub received_at: f32,
}

#[derive(Resource, Debug, Default)]
pub struct ContractFeedback(pub Option<ContractResult>);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuildButton {
    Sell(ItemDefinitionId, u32),
    Accept(u32),
    Abandon,
}

#[derive(Component)]
struct ContractHud;

#[derive(Component)]
struct ContractHudText;

pub struct GuildPlugin;

impl Plugin for GuildPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownGuildPrices>()
            .init_resource::<KnownContracts>()
            .init_resource::<ContractFeedback>()
            .add_systems(Startup, spawn_contract_hud)
            .add_systems(
                Update,
                (
                    handle_guild_messages,
                    handle_guild_clicks,
                    update_contract_hud,
                    auto_guild,
                ),
            );
    }
}

fn handle_guild_messages(
    time: Res<Time>,
    mut prices: EventReader<ClientReceiveMessage<GuildPrices>>,
    mut contracts: EventReader<ClientReceiveMessage<ContractsSnapshot>>,
    mut results: EventReader<ClientReceiveMessage<ContractResult>>,
    mut known_prices: ResMut<KnownGuildPrices>,
    mut known_contracts: ResMut<KnownContracts>,
    mut feedback: ResMut<ContractFeedback>,
) {
    for event in prices.read() {
        known_prices.0 = Some(event.message().clone());
    }
    for event in contracts.read() {
        known_contracts.snapshot = Some(event.message().clone());
        known_contracts.received_at = time.elapsed_secs();
    }
    for event in results.read() {
        let result = event.message().clone();
        info!(success = result.success, reason = %result.reason, "contrato");
        feedback.0 = Some(result);
    }
}

fn handle_guild_clicks(
    buttons: Query<(&Interaction, &GuildButton), Changed<Interaction>>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let _ = match *button {
            GuildButton::Sell(item, quantity) => connection_manager
                .send_message::<ReliableChannel, _>(&SellToGuild { item, quantity }),
            GuildButton::Accept(id) => {
                connection_manager.send_message::<ReliableChannel, _>(&AcceptContract { id })
            }
            GuildButton::Abandon => {
                connection_manager.send_message::<ReliableChannel, _>(&AbandonContract)
            }
        };
    }
}

/// Dev (§39): MARVYR_AUTOGUILD=1, atracado, deposita o porão, troca
/// tudo com a guilda, aceita a primeira oferta e desatraca — smoke do loop.
fn auto_guild(
    time: Res<Time>,
    docked: Res<crate::net::MyDocked>,
    storage: Res<crate::port_screen::KnownPortStorage>,
    contracts: Res<KnownContracts>,
    mut timer: Local<f32>,
    mut step: Local<u8>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if std::env::var_os("MARVYR_AUTOGUILD").is_none() || !docked.0 {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 1.5 {
        return;
    }
    *timer = 0.0;
    let _ = match *step {
        0 => connection_manager.send_message::<ReliableChannel, _>(&StorageDepositAll),
        1 => {
            for line in &storage.0 {
                let _ = connection_manager.send_message::<ReliableChannel, _>(&SellToGuild {
                    item: line.item,
                    quantity: line.quantity,
                });
            }
            Ok(())
        }
        2 => match contracts.snapshot.as_ref().and_then(|s| s.offers.first()) {
            Some(offer) => connection_manager
                .send_message::<ReliableChannel, _>(&AcceptContract { id: offer.id }),
            None => return,
        },
        3 => connection_manager.send_message::<ReliableChannel, _>(&Undock),
        _ => return,
    };
    *step += 1;
}

/// Contrato ativo com o tempo restante descontado localmente.
pub fn active_contract(known: &KnownContracts, now: f32) -> Option<ContractLine> {
    let mut line = known.snapshot.as_ref()?.active.clone()?;
    let elapsed = (now - known.received_at).max(0.0) as u32;
    line.remaining_secs = line.remaining_secs.saturating_sub(elapsed);
    Some(line)
}

fn clock(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn progress_label(line: &ContractLine) -> String {
    if line.hunt {
        trf(
            "abates {0}/{1}",
            &[&line.progress.to_string(), &line.target.to_string()],
        )
    } else {
        tr("entregue ao atracar no destino")
    }
}

// ===== Aba Guilda =====

#[derive(Debug, Clone, PartialEq)]
pub struct GuildRow {
    pub item: ItemDefinitionId,
    pub name: String,
    pub stored: u32,
    pub in_cargo: u32,
    /// Quanto 10 unidades rendem aqui (0 = a guilda daqui não aceita).
    pub here: u32,
    /// Quanto rendem no outro porto, no recurso de lá.
    pub other: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildView {
    /// Recurso com que a guilda deste porto paga.
    pub payout: String,
    pub other_port: String,
    pub other_payout: String,
    pub rows: Vec<GuildRow>,
}

fn quantity_of(lines: &[StorageLine], item: ItemDefinitionId) -> u32 {
    lines
        .iter()
        .filter(|line| line.item == item)
        .map(|line| line.quantity)
        .sum()
}

/// Uma linha por item aceito: quanto 10 rendem aqui e no outro porto.
pub fn guild_view(prices: Option<&GuildPrices>, storage: &[StorageLine], here: &str) -> GuildView {
    let Some(prices) = prices else {
        return GuildView {
            payout: String::new(),
            other_port: String::new(),
            other_payout: String::new(),
            rows: Vec::new(),
        };
    };
    let here_index = prices
        .ports
        .iter()
        .position(|port| port == here)
        .unwrap_or(0);
    let other_index = (0..prices.ports.len()).find(|index| *index != here_index);
    let payout_of = |index: Option<usize>| {
        index
            .and_then(|index| prices.payouts.get(index))
            .map(|name| tr(name))
            .unwrap_or_default()
    };
    GuildView {
        payout: payout_of(Some(here_index)),
        other_port: other_index
            .map(|index| tr(&prices.ports[index]))
            .unwrap_or_default(),
        other_payout: payout_of(other_index),
        rows: prices
            .lines
            .iter()
            .map(|line| GuildRow {
                item: line.item,
                name: tr(&line.item_name),
                stored: quantity_of(storage, line.item),
                in_cargo: quantity_of(&prices.cargo, line.item),
                here: line.per_ten.get(here_index).copied().unwrap_or(0),
                other: other_index
                    .and_then(|index| line.per_ten.get(index).copied())
                    .unwrap_or(0),
            })
            .collect(),
    }
}

fn cell(parent: &mut ChildBuilder, value: impl Into<String>, width: f32, color: Color) {
    parent.spawn((
        ui::text(value, 13.0, color),
        Node {
            width: Val::Px(width),
            ..default()
        },
    ));
}

fn guild_button(parent: &mut ChildBuilder, label: &str, action: GuildButton, color: Color) {
    parent
        .spawn((ui::button(Node::default(), ui::BUTTON_BG), action))
        .with_children(|b| {
            b.spawn(ui::text(label, 13.0, color));
        });
}

pub fn spawn_guild_body(parent: &mut ChildBuilder, view: &GuildView) {
    if view.rows.is_empty() {
        parent.spawn(ui::text("GUILDA MERCANTE", 12.0, ui::PANEL_BORDER));
        parent.spawn(ui::text(
            "Aguardando a tabela da guilda...",
            13.0,
            ui::TEXT_DIM,
        ));
        return;
    }
    parent.spawn(ui::text(
        trf(
            "GUILDA MERCANTE - paga em {0}, do armazém deste porto (o item entregue é destruído)",
            &[&view.payout],
        ),
        12.0,
        ui::PANEL_BORDER,
    ));
    parent
        .spawn(Node {
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|header| {
            cell(header, "Item", 150.0, ui::TEXT_DIM);
            cell(header, "Armazém", 70.0, ui::TEXT_DIM);
            cell(header, "Porão", 60.0, ui::TEXT_DIM);
            cell(header, "10 rendem aqui", 130.0, ui::TEXT_DIM);
            cell(header, view.other_port.as_str(), 150.0, ui::TEXT_DIM);
        });
    for row in &view.rows {
        parent
            .spawn(Node {
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|line| {
                cell(line, row.name.as_str(), 150.0, ui::TEXT);
                cell(line, row.stored.to_string(), 70.0, ui::TEXT);
                cell(line, row.in_cargo.to_string(), 60.0, ui::TEXT_DIM);
                let rate = |amount: u32, payout: &str| {
                    if amount == 0 {
                        String::from("—")
                    } else {
                        format!("{amount} {payout}")
                    }
                };
                cell(line, rate(row.here, &view.payout), 130.0, ui::GOLD);
                cell(
                    line,
                    rate(row.other, &view.other_payout),
                    150.0,
                    ui::TEXT_DIM,
                );
                if row.stored > 0 && row.here > 0 {
                    guild_button(line, "Trocar 1", GuildButton::Sell(row.item, 1), ui::GOLD);
                    guild_button(line, "10", GuildButton::Sell(row.item, 10), ui::GOLD);
                    guild_button(
                        line,
                        "Tudo",
                        GuildButton::Sell(row.item, row.stored),
                        ui::GOLD,
                    );
                }
            });
    }
    parent.spawn(ui::text(
        "Trocar muito derruba a taxa; ela se recupera com o tempo. Deposite o porão para trocar.",
        11.0,
        ui::TEXT_DIM,
    ));
}

// ===== Aba Contratos =====

/// Recompensa do contrato: recurso bruto no armazém do porto.
fn reward_label(line: &ContractLine) -> String {
    format!("{} {}", line.reward_quantity, tr(&line.reward_item))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContractsView {
    pub offers: Vec<ContractLine>,
    pub active: Option<ContractLine>,
}

pub fn contracts_view(known: &KnownContracts, now: f32) -> ContractsView {
    ContractsView {
        offers: known
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.offers.clone())
            .unwrap_or_default(),
        active: active_contract(known, now),
    }
}

pub fn spawn_contracts_body(parent: &mut ChildBuilder, view: &ContractsView) {
    parent.spawn(ui::text("SEU CONTRATO", 12.0, ui::PANEL_BORDER));
    match &view.active {
        Some(active) => {
            parent
                .spawn(Node {
                    column_gap: Val::Px(10.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|line| {
                    line.spawn(ui::text(tr(&active.title), 14.0, ui::TEXT));
                    line.spawn(ui::text(reward_label(active), 14.0, ui::GOLD));
                    line.spawn(ui::text(
                        trf(
                            "{0} restantes - {1}",
                            &[&clock(active.remaining_secs), &progress_label(active)],
                        ),
                        13.0,
                        ui::AMBER,
                    ));
                    guild_button(line, "Abandonar", GuildButton::Abandon, ui::DANGER);
                });
        }
        None => {
            parent.spawn(ui::text("Nenhum contrato ativo.", 13.0, ui::TEXT_DIM));
        }
    }
    parent.spawn((
        ui::text("QUADRO DE CONTRATOS", 12.0, ui::PANEL_BORDER),
        Node {
            margin: UiRect::top(Val::Px(10.0)),
            ..default()
        },
    ));
    if view.offers.is_empty() {
        parent.spawn(ui::text(
            "Quadro vazio - volte mais tarde.",
            13.0,
            ui::TEXT_DIM,
        ));
    }
    for offer in &view.offers {
        parent
            .spawn(Node {
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|line| {
                line.spawn((
                    ui::text(tr(&offer.title), 14.0, ui::TEXT),
                    Node {
                        width: Val::Px(430.0),
                        ..default()
                    },
                ));
                cell(line, reward_label(offer), 110.0, ui::GOLD);
                cell(
                    line,
                    format!("{} min", offer.duration_secs / 60),
                    60.0,
                    ui::TEXT_DIM,
                );
                if view.active.is_none() {
                    guild_button(line, "Aceitar", GuildButton::Accept(offer.id), ui::OK_GREEN);
                }
            });
    }
    parent.spawn(ui::text(
        "1 contrato por vez. Entrega: a carga vai no porão e pode ser saqueada no caminho.",
        11.0,
        ui::TEXT_DIM,
    ));
}

// ===== HUD do mar =====

/// Linha do contrato ativo sob o painel do navio (topo-esquerda).
fn spawn_contract_hud(mut commands: Commands) {
    commands
        .spawn((
            ui::panel(Node {
                position_type: PositionType::Absolute,
                left: Val::Px(ui::MARGIN),
                top: Val::Px(ui::MARGIN + 132.0),
                display: Display::None,
                ..default()
            }),
            SeaHud,
            ContractHud,
        ))
        .with_children(|panel| {
            panel.spawn((ui::text("", 12.0, ui::AMBER), ContractHudText));
        });
}

fn update_contract_hud(
    time: Res<Time>,
    known: Res<KnownContracts>,
    mut huds: Query<&mut Node, With<ContractHud>>,
    mut texts: Query<&mut Text, With<ContractHudText>>,
) {
    let active = active_contract(&known, time.elapsed_secs());
    let display = if active.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut huds {
        if node.display != display {
            node.display = display;
        }
    }
    let Some(active) = active else {
        return;
    };
    let value = trf(
        "CONTRATO: {0}\n{1} - {2} - {3}",
        &[
            &tr(&active.title),
            &clock(active.remaining_secs),
            &progress_label(&active),
            &reward_label(&active),
        ],
    );
    for mut text in &mut texts {
        if text.0 != value {
            text.0 = value.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use marvyr_protocol::GuildPriceLine;

    use super::*;

    #[test]
    fn guild_view_shows_the_rate_here_and_at_the_other_port() {
        let ore = ItemDefinitionId::new();
        let prices = GuildPrices {
            ports: vec![
                String::from("Porto da Serra"),
                String::from("Porto da Mina"),
            ],
            payouts: vec![String::from("Madeira"), String::from("Minério")],
            lines: vec![GuildPriceLine {
                item: ore,
                item_name: String::from("Minério"),
                per_ten: vec![13, 0],
            }],
            cargo: vec![StorageLine {
                item: ore,
                item_name: String::from("Minério"),
                quantity: 3,
                instance: None,
                quality: None,
            }],
        };
        let storage = vec![StorageLine {
            item: ore,
            item_name: String::from("Minério"),
            quantity: 40,
            instance: None,
            quality: None,
        }];

        let view = guild_view(Some(&prices), &storage, "Porto da Serra");
        assert_eq!(
            (view.payout.as_str(), view.other_port.as_str()),
            ("Madeira", "Porto da Mina")
        );
        let row = &view.rows[0];
        assert_eq!((row.here, row.other), (13, 0));
        assert_eq!((row.stored, row.in_cargo), (40, 3));

        let view = guild_view(Some(&prices), &storage, "Porto da Mina");
        assert_eq!(view.payout, "Minério");
        assert_eq!((view.rows[0].here, view.rows[0].other), (0, 13));
    }

    #[test]
    fn active_contract_counts_down_locally() {
        let known = KnownContracts {
            snapshot: Some(ContractsSnapshot {
                offers: Vec::new(),
                active: Some(ContractLine {
                    id: 1,
                    title: String::from("Caca"),
                    reward_item: String::from("Madeira"),
                    reward_quantity: 30,
                    duration_secs: 600,
                    remaining_secs: 100,
                    progress: 1,
                    target: 2,
                    hunt: true,
                }),
            }),
            received_at: 10.0,
        };
        assert_eq!(active_contract(&known, 40.0).unwrap().remaining_secs, 70);
        assert_eq!(active_contract(&known, 500.0).unwrap().remaining_secs, 0);
        assert_eq!(clock(70), "1:10");
    }
}
