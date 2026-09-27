//! Mercado no client (PRD MF-023..026), por escambo. O client vê catálogo e
//! o quadro de ofertas; storage e execução são do servidor (Pilar 4). O
//! painel (MF-040, MF-058) vive na aba Mercado da tela de porto, em bevy_ui:
//! lista ofertas, cria/cancela/aceita por teclado ou mouse, e o veredito
//! `MarketResult` aparece na linha de status da tela de porto. Z/X/V/N/B
//! continuam como atalhos dev (escondidos do HUD desde MF-043).

use std::collections::HashMap;

use crate::net::{MyDocked, ReliableChannel};
use crate::port_screen::{PortScreenState, PortTab};
use crate::ui;
use bevy::ecs::prelude::*;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_protocol::{
    BuySellOrder, CancelSellOrder, CatalogSnapshot, CreateSellOrder, ItemLine, MarketResult,
    OrderLine, OrdersSnapshot, StorageDepositAll, StorageLine, StorageWithdrawAll,
};
use marvyr_shared::ids::ItemDefinitionId;

/// Catálogo do servidor: nome → id real (para os intents) + peso (UI).
#[derive(Resource, Debug, Default)]
pub struct KnownCatalog(pub HashMap<String, ItemLine>);

/// Quadro de orders conhecido (último snapshot do servidor).
#[derive(Resource, Debug, Default)]
pub struct KnownOrders(pub Vec<marvyr_protocol::OrderLine>);

/// Estado local do formulário de oferta ("dou X por Y") e da seleção.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MarketForm {
    pub item_index: usize,
    /// v53: qual peça do armazém (equipamento do mesmo tipo difere).
    pub piece_index: usize,
    pub quantity: String,
    pub ask_index: usize,
    pub ask_quantity: String,
    pub selected_order: usize,
    pub focus: FormFocus,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FormFocus {
    #[default]
    Orders,
    Item,
    Quantity,
    AskItem,
    AskQuantity,
}

/// Último veredito do servidor, exibido no painel.
#[derive(Resource, Debug, Default)]
pub struct MarketFeedback(pub Option<MarketResult>);

/// Botões do painel de mercado (mouse); mesmo efeito das teclas.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketButton {
    SelectOrder(usize),
    ExecuteOrder(usize),
    Focus(FormFocus),
    /// Troca o item do campo (`Item` ou `AskItem`).
    Pick(FormFocus, i8),
    Step(FormFocus, i8),
    /// v53: troca a peça oferecida (equipamento).
    Piece(i8),
    Submit,
}

pub struct MarketPlugin;

impl Plugin for MarketPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownCatalog>()
            .init_resource::<KnownOrders>()
            .init_resource::<MarketForm>()
            .init_resource::<MarketFeedback>()
            .add_systems(
                Update,
                (
                    handle_catalog_snapshot,
                    handle_orders_snapshot,
                    handle_market_result,
                    handle_market_panel_input,
                    handle_market_clicks,
                ),
            );
    }
}

fn handle_catalog_snapshot(
    mut catalog_events: EventReader<ClientReceiveMessage<CatalogSnapshot>>,
    mut known: ResMut<KnownCatalog>,
    mut form: ResMut<MarketForm>,
) {
    for event in catalog_events.read() {
        known.0 = event
            .message()
            .items
            .iter()
            .map(|line| (line.name.clone(), line.clone()))
            .collect();
        let last = known.0.len().saturating_sub(1);
        form.item_index = form.item_index.min(last);
        form.ask_index = form.ask_index.min(last);
        info!(items = known.0.len(), "catálogo de itens recebido");
    }
}

fn handle_market_result(
    mut events: EventReader<ClientReceiveMessage<MarketResult>>,
    mut feedback: ResMut<MarketFeedback>,
) {
    for event in events.read() {
        let result = event.message();
        feedback.0 = Some(result.clone());
        if result.success {
            info!(reason = %result.reason, "mercado: ok");
        } else {
            warn!(reason = %result.reason, "mercado: recusado");
        }
    }
}

/// Quadro de orders: guarda e redesenha o painel.
fn handle_orders_snapshot(
    mut orders_events: EventReader<ClientReceiveMessage<OrdersSnapshot>>,
    mut known: ResMut<KnownOrders>,
    mut form: ResMut<MarketForm>,
) {
    for event in orders_events.read() {
        known.0 = sorted_orders(event.message().orders.clone());
        form.selected_order = form.selected_order.min(known.0.len().saturating_sub(1));
    }
}

/// Ordena o quadro para a UI: minhas primeiro, depois por item oferecido.
fn sorted_orders(mut orders: Vec<OrderLine>) -> Vec<OrderLine> {
    orders.sort_by(|a, b| {
        (!a.mine, &a.item_name, a.order_num).cmp(&(!b.mine, &b.item_name, b.order_num))
    });
    orders
}

/// Peças do armazém deste porto do tipo `item` (equipamento vem peça a
/// peça; recurso agregado não entra).
fn pieces(storage: &[StorageLine], item: Option<ItemDefinitionId>) -> Vec<&StorageLine> {
    storage
        .iter()
        .filter(|line| line.instance.is_some() && Some(line.item) == item)
        .collect()
}

fn offered_item(form: &MarketForm, catalog: &KnownCatalog) -> Option<ItemDefinitionId> {
    catalog_items(catalog)
        .get(form.item_index)
        .map(|line| line.id)
}

/// A peça escolhida (se o item oferecido é equipamento no armazém).
fn chosen_piece<'a>(
    form: &MarketForm,
    catalog: &KnownCatalog,
    storage: &'a [StorageLine],
) -> Option<&'a StorageLine> {
    let pieces = pieces(storage, offered_item(form, catalog));
    (!pieces.is_empty()).then(|| pieces[form.piece_index % pieces.len()])
}

/// "Raro · 3 afixos" (o que distingue peças do mesmo tipo).
fn quality_label(quality: Option<&marvyr_domain_items::Quality>) -> String {
    let rarity = crate::affixes::quality_rarity(quality);
    let affixes = quality.map_or(0, |q| q.affixes.len() + q.gems.len());
    format!(
        "{} · {}",
        crate::i18n::tr(crate::affixes::rarity_label(rarity)),
        crate::i18n::trf("{0} afixos", &[&affixes.to_string()])
    )
}

fn catalog_items(catalog: &KnownCatalog) -> Vec<&ItemLine> {
    let mut items: Vec<_> = catalog.0.values().collect();
    items.sort_by_key(|line| line.name.as_str());
    items
}

/// O que a aba Mercado mostra; a tela de porto reconstrói o corpo quando muda.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketView {
    pub orders: Vec<(String, &'static str)>,
    pub selected: usize,
    pub focus: FormFocus,
    pub item: String,
    /// v53: a peça escolhida, quando o item é equipamento no armazém.
    pub piece: Option<String>,
    pub quantity: String,
    pub ask_item: String,
    pub ask_quantity: String,
}

pub fn market_view(
    form: &MarketForm,
    orders: &[OrderLine],
    catalog: &KnownCatalog,
    storage: &[StorageLine],
) -> MarketView {
    let items = catalog_items(catalog);
    let name = |index: usize| {
        items
            .get(index)
            .map(|line| crate::i18n::tr(&line.name))
            .unwrap_or_else(|| String::from("—"))
    };
    let or_dash = |value: &str| {
        if value.is_empty() {
            String::from("—")
        } else {
            value.to_owned()
        }
    };
    MarketView {
        orders: orders
            .iter()
            .map(|order| (order_label(order), order_action_label(order)))
            .collect(),
        selected: form.selected_order,
        focus: form.focus,
        item: name(form.item_index),
        piece: chosen_piece(form, catalog, storage)
            .map(|line| quality_label(line.quality.as_ref())),
        quantity: or_dash(&form.quantity),
        ask_item: name(form.ask_index),
        ask_quantity: or_dash(&form.ask_quantity),
    }
}

/// Linha de oferta: número, o que dá, o que pede, região, dono.
fn order_label(order: &OrderLine) -> String {
    let mine = if order.mine {
        format!(" [{}]", crate::i18n::tr("MINHA"))
    } else {
        String::new()
    };
    // v53: peça com raridade diz qual é (o comprador vê o que leva).
    let piece = order
        .quality
        .as_ref()
        .map(|quality| format!(" ({})", quality_label(Some(quality))))
        .unwrap_or_default();
    format!(
        "#{:<3} {:>3} {}{} {} {} {}  {}{}",
        order.order_num,
        order.quantity,
        crate::i18n::tr(&order.item_name),
        piece,
        crate::i18n::tr("por"),
        order.ask_quantity,
        crate::i18n::tr(&order.ask_item_name),
        crate::i18n::tr(&order.region),
        mine,
    )
}

fn order_action_label(order: &OrderLine) -> &'static str {
    if order.mine {
        "Cancelar"
    } else {
        "Trocar"
    }
}

fn selected_bg(selected: bool) -> Color {
    if selected {
        ui::BUTTON_SELECTED
    } else {
        ui::BUTTON_BG
    }
}

fn small_button(parent: &mut ChildBuilder, label: &str, action: MarketButton) {
    parent
        .spawn((
            ui::button(
                Node {
                    min_width: Val::Px(30.0),
                    ..default()
                },
                ui::BUTTON_BG,
            ),
            action,
        ))
        .with_children(|b| {
            b.spawn(ui::text(label, 13.0, ui::TEXT));
        });
}

fn form_row(parent: &mut ChildBuilder, view: &MarketView, label: &str, field: FormFocus) {
    let value = match field {
        FormFocus::Item => view.item.as_str(),
        FormFocus::Quantity => view.quantity.as_str(),
        FormFocus::AskItem => view.ask_item.as_str(),
        _ => view.ask_quantity.as_str(),
    };
    let (minus, plus, minus_label, plus_label) = match field {
        FormFocus::Item | FormFocus::AskItem => (
            MarketButton::Pick(field, -1),
            MarketButton::Pick(field, 1),
            "<",
            ">",
        ),
        _ => (
            MarketButton::Step(field, -1),
            MarketButton::Step(field, 1),
            "-",
            "+",
        ),
    };
    parent
        .spawn(Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                ui::text(label, 12.0, ui::TEXT_DIM),
                Node {
                    width: Val::Px(48.0),
                    ..default()
                },
            ));
            small_button(row, minus_label, minus);
            row.spawn((
                ui::button(
                    Node {
                        flex_grow: 1.0,
                        justify_content: JustifyContent::Start,
                        ..default()
                    },
                    selected_bg(view.focus == field),
                ),
                MarketButton::Focus(field),
            ))
            .with_children(|b| {
                b.spawn(ui::text(value, 13.0, ui::TEXT));
            });
            small_button(row, plus_label, plus);
        });
}

/// v53: "Peça  < Raro · 3 afixos >" logo abaixo do item oferecido.
fn piece_row(parent: &mut ChildBuilder, piece: &str) {
    parent
        .spawn(Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                ui::text("Peça", 12.0, ui::TEXT_DIM),
                Node {
                    width: Val::Px(48.0),
                    ..default()
                },
            ));
            small_button(row, "<", MarketButton::Piece(-1));
            row.spawn((
                ui::text(piece, 13.0, ui::GOLD),
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            ));
            small_button(row, ">", MarketButton::Piece(1));
        });
}

/// Corpo da aba Mercado: ofertas à esquerda, formulário de oferta à direita.
pub fn spawn_market_body(parent: &mut ChildBuilder, view: &MarketView) {
    parent
        .spawn(Node {
            column_gap: Val::Px(16.0),
            flex_grow: 1.0,
            ..default()
        })
        .with_children(|columns| {
            columns
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    row_gap: Val::Px(4.0),
                    overflow: Overflow::clip_y(),
                    ..default()
                })
                .with_children(|list| {
                    list.spawn(ui::text("OFERTAS DE TROCA", 12.0, ui::PANEL_BORDER));
                    if view.orders.is_empty() {
                        list.spawn(ui::text("Mercado: sem ofertas", 13.0, ui::TEXT_DIM));
                    }
                    // ponytail: sem rolagem; adicionar scroll quando houver mais orders que a tela aguenta.
                    for (index, (label, action)) in view.orders.iter().enumerate() {
                        let selected = index == view.selected && view.focus == FormFocus::Orders;
                        list.spawn(Node {
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((
                                ui::button(
                                    Node {
                                        flex_grow: 1.0,
                                        justify_content: JustifyContent::Start,
                                        ..default()
                                    },
                                    selected_bg(selected),
                                ),
                                MarketButton::SelectOrder(index),
                            ))
                            .with_children(|b| {
                                b.spawn(ui::text(label.as_str(), 13.0, ui::TEXT));
                            });
                            row.spawn((
                                ui::button(Node::default(), ui::BUTTON_BG),
                                MarketButton::ExecuteOrder(index),
                            ))
                            .with_children(|b| {
                                b.spawn(ui::text(*action, 13.0, ui::GOLD));
                            });
                        });
                    }
                });
            columns
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Px(300.0),
                    flex_shrink: 0.0,
                    row_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|form| {
                    form.spawn(ui::text("OFERECER TROCA", 12.0, ui::PANEL_BORDER));
                    form_row(form, view, "Dou", FormFocus::Item);
                    if let Some(piece) = &view.piece {
                        piece_row(form, piece);
                    }
                    form_row(form, view, "Qtd", FormFocus::Quantity);
                    form_row(form, view, "Por", FormFocus::AskItem);
                    form_row(form, view, "Qtd", FormFocus::AskQuantity);
                    form.spawn((
                        ui::button(Node::default(), ui::BUTTON_SELECTED),
                        MarketButton::Submit,
                    ))
                    .with_children(|b| {
                        b.spawn(ui::text("Criar oferta", 13.0, ui::GOLD));
                    });
                    form.spawn(ui::text(
                        "Sai do armazém deste porto. Quem aceita entrega o pedido inteiro.",
                        11.0,
                        ui::TEXT_DIM,
                    ));
                    form.spawn(ui::text(
                        "Clique no campo e digite · Shift+clique: ±10 · Enter envia",
                        11.0,
                        ui::TEXT_DIM,
                    ));
                });
        });
}

/// Intenção de mercado produzida pelo painel (testável sem rede).
#[derive(Debug, Clone, PartialEq, Eq)]
enum MarketIntent {
    Create(CreateSellOrder),
    Cancel(CancelSellOrder),
    Buy(BuySellOrder),
}

/// Sem validação local: o servidor é a lei e responde via `MarketResult`.
fn form_create_intent(
    form: &MarketForm,
    catalog: &KnownCatalog,
    storage: &[StorageLine],
) -> Option<MarketIntent> {
    let items = catalog_items(catalog);
    Some(MarketIntent::Create(CreateSellOrder {
        item: items.get(form.item_index)?.id,
        quantity: form.quantity.parse::<u32>().unwrap_or_default(),
        ask_item: items.get(form.ask_index)?.id,
        ask_quantity: form.ask_quantity.parse::<u32>().unwrap_or_default(),
        instance: chosen_piece(form, catalog, storage).and_then(|line| line.instance),
    }))
}

fn order_intent(order: &OrderLine) -> MarketIntent {
    if order.mine {
        MarketIntent::Cancel(CancelSellOrder {
            order_num: order.order_num,
        })
    } else {
        MarketIntent::Buy(BuySellOrder {
            order_num: order.order_num,
        })
    }
}

fn next_focus(focus: FormFocus) -> FormFocus {
    match focus {
        FormFocus::Orders => FormFocus::Item,
        FormFocus::Item => FormFocus::Quantity,
        FormFocus::Quantity => FormFocus::AskItem,
        FormFocus::AskItem => FormFocus::AskQuantity,
        FormFocus::AskQuantity => FormFocus::Orders,
    }
}

/// Teclado do painel: Tab troca campo, setas escolhem, Enter envia.
/// Só vale com a aba Mercado aberta (antes digitava/comprava até no mar).
#[allow(clippy::too_many_arguments)]
pub fn handle_market_panel_input(
    mut keyboard: EventReader<KeyboardInput>,
    docked: Res<MyDocked>,
    state: Res<PortScreenState>,
    mut form: ResMut<MarketForm>,
    catalog: Res<KnownCatalog>,
    orders: Res<KnownOrders>,
    storage: Res<crate::port_screen::KnownPortStorage>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if !docked.0 || state.active_tab != PortTab::Market {
        keyboard.clear();
        return;
    }
    let mut intent = None;
    for event in keyboard.read() {
        if event.state != ButtonState::Pressed || event.repeat {
            continue;
        }
        match &event.logical_key {
            Key::Tab => form.focus = next_focus(form.focus),
            Key::ArrowUp => {
                if form.focus == FormFocus::Orders {
                    form.selected_order = form.selected_order.saturating_sub(1);
                }
            }
            Key::ArrowDown => {
                if form.focus == FormFocus::Orders {
                    form.selected_order = form
                        .selected_order
                        .saturating_add(1)
                        .min(orders.0.len().saturating_sub(1));
                }
            }
            Key::ArrowLeft => pick_item(&mut form, -1, &catalog),
            Key::ArrowRight => pick_item(&mut form, 1, &catalog),
            Key::Enter => {
                intent = match form.focus {
                    FormFocus::Orders => orders.0.get(form.selected_order).map(order_intent),
                    _ => form_create_intent(&form, &catalog, &storage.0),
                };
            }
            Key::Character(chars) => {
                if let Some(digit) = chars.chars().next().filter(|digit| digit.is_ascii_digit()) {
                    match form.focus {
                        FormFocus::Quantity => form.quantity.push(digit),
                        FormFocus::AskQuantity => form.ask_quantity.push(digit),
                        _ => {}
                    }
                }
            }
            Key::Backspace => match form.focus {
                FormFocus::Quantity => {
                    form.quantity.pop();
                }
                FormFocus::AskQuantity => {
                    form.ask_quantity.pop();
                }
                _ => {}
            },
            _ => {}
        }
    }
    if let Some(intent) = intent {
        send_intent(&mut connection_manager, intent);
    }
}

fn send_intent(connection_manager: &mut ConnectionManager, intent: MarketIntent) {
    let _ = match intent {
        MarketIntent::Create(message) => {
            connection_manager.send_message::<ReliableChannel, _>(&message)
        }
        MarketIntent::Cancel(message) => {
            connection_manager.send_message::<ReliableChannel, _>(&message)
        }
        MarketIntent::Buy(message) => {
            connection_manager.send_message::<ReliableChannel, _>(&message)
        }
    };
}

/// Troca o item do campo em foco (`Item` ou `AskItem`) por `delta`.
fn pick_item(form: &mut MarketForm, delta: i8, catalog: &KnownCatalog) {
    let last = catalog_items(catalog).len().saturating_sub(1);
    let index = match form.focus {
        FormFocus::Item => &mut form.item_index,
        FormFocus::AskItem => &mut form.ask_index,
        _ => return,
    };
    *index = index.saturating_add_signed(isize::from(delta)).min(last);
}

/// Soma `delta` a um campo numérico do formulário, sem ficar negativo.
fn step_field(value: &str, delta: i64) -> String {
    let current = value.parse::<i64>().unwrap_or_default();
    current.saturating_add(delta).max(0).to_string()
}

/// Efeito de um clique no painel: mesmas transições do teclado.
fn apply_market_button(
    form: &mut MarketForm,
    button: MarketButton,
    shift: bool,
    catalog: &KnownCatalog,
    storage: &[StorageLine],
    orders: &[OrderLine],
) -> Option<MarketIntent> {
    let scale: i64 = if shift { 10 } else { 1 };
    match button {
        MarketButton::SelectOrder(index) => {
            form.focus = FormFocus::Orders;
            form.selected_order = index;
        }
        MarketButton::ExecuteOrder(index) => {
            form.focus = FormFocus::Orders;
            form.selected_order = index;
            return orders.get(index).map(order_intent);
        }
        MarketButton::Focus(field) => form.focus = field,
        MarketButton::Pick(field, delta) => {
            form.focus = field;
            pick_item(form, delta, catalog);
        }
        MarketButton::Step(field, delta) => {
            form.focus = field;
            let delta = i64::from(delta) * scale;
            match field {
                FormFocus::Quantity => form.quantity = step_field(&form.quantity, delta),
                FormFocus::AskQuantity => form.ask_quantity = step_field(&form.ask_quantity, delta),
                _ => {}
            }
        }
        MarketButton::Piece(delta) => {
            let count = pieces(storage, offered_item(form, catalog)).len().max(1);
            let index = (form.piece_index % count) as isize + isize::from(delta);
            form.piece_index = index.rem_euclid(count as isize) as usize;
        }
        MarketButton::Submit => return form_create_intent(form, catalog, storage),
    }
    None
}

pub fn handle_market_clicks(
    buttons: Query<(&Interaction, &MarketButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut form: ResMut<MarketForm>,
    catalog: Res<KnownCatalog>,
    orders: Res<KnownOrders>,
    storage: Res<crate::port_screen::KnownPortStorage>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(intent) =
            apply_market_button(&mut form, *button, shift, &catalog, &storage.0, &orders.0)
        {
            send_intent(&mut connection_manager, intent);
        }
    }
}

/// Z/X/V/N/B — a interface de mercado do slice (§45: você opera no porto
/// onde está; o servidor recusa o resto). MARVYR_AUTOMARKET=1 faz o
/// ciclo depositar → ofertar → aceitar → retirar sozinho (§39).
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
pub fn send_market_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    known_catalog: Res<KnownCatalog>,
    known_orders: Res<KnownOrders>,
    docked: Res<crate::net::MyDocked>,
    mut auto_timer: Local<f32>,
    mut auto_step: Local<u8>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    // Letras do mercado só valem atracado: no mar, B é o farol (v46).
    let key = |code| docked.0 && keys.just_pressed(code);
    let manual_deposit = key(KeyCode::KeyZ);
    let manual_withdraw = key(KeyCode::KeyX);
    let manual_sell = key(KeyCode::KeyV);
    let manual_cancel = key(KeyCode::KeyN);
    let manual_buy = key(KeyCode::KeyB);

    let mut auto = None;
    if automarket_enabled() {
        *auto_timer += time.delta_secs();
        if *auto_timer >= 2.5 {
            *auto_timer = 0.0;
            auto = Some(match *auto_step {
                0 => AutoStep::Deposit,
                1 => AutoStep::Sell,
                2 => AutoStep::Buy,
                _ => AutoStep::Withdraw,
            });
            *auto_step = (*auto_step + 1) % 4;
        }
    }

    if manual_deposit || auto.is_some_and(|step| step == AutoStep::Deposit) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&StorageDepositAll);
    }
    if manual_withdraw || auto.is_some_and(|step| step == AutoStep::Withdraw) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&StorageWithdrawAll);
    }
    if manual_sell || auto.is_some_and(|step| step == AutoStep::Sell) {
        // Dev: oferece 10 Madeira por 5 Minério.
        if let (Some(wood), Some(ore)) = (
            known_catalog.0.get("Madeira"),
            known_catalog.0.get("Minério"),
        ) {
            let intent = CreateSellOrder {
                item: wood.id,
                quantity: 10,
                ask_item: ore.id,
                ask_quantity: 5,
                instance: None,
            };
            let _ = connection_manager.send_message::<ReliableChannel, _>(&intent);
        }
    }
    if manual_cancel {
        // Cancela a order sua mais antiga que ainda está no quadro.
        if let Some(order) = known_orders.0.iter().find(|order| order.mine) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(&CancelSellOrder {
                order_num: order.order_num,
            });
        }
    }
    if manual_buy || auto.is_some_and(|step| step == AutoStep::Buy) {
        // Aceita a primeira oferta alheia (o servidor valida porto e
        // pagamento, §44/§45).
        if let Some(order) = known_orders.0.iter().find(|order| !order.mine) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(&BuySellOrder {
                order_num: order.order_num,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoStep {
    Deposit,
    Sell,
    Buy,
    Withdraw,
}

fn automarket_enabled() -> bool {
    std::env::var_os("MARVYR_AUTOMARKET").is_some()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};

    use super::*;

    fn line(id: ItemDefinitionId, name: &str) -> ItemLine {
        ItemLine {
            id,
            name: String::from(name),
            weight: 2,
            equipment_slot: None,
        }
    }

    fn two_items() -> (KnownCatalog, ItemDefinitionId, ItemDefinitionId) {
        let (wood, ore) = (ItemDefinitionId::new(), ItemDefinitionId::new());
        let catalog = KnownCatalog(HashMap::from([
            (String::from("Madeira"), line(wood, "Madeira")),
            (String::from("Minério"), line(ore, "Minério")),
        ]));
        (catalog, wood, ore)
    }

    fn order(order_num: u32, item_name: &str, quantity: u32, mine: bool) -> OrderLine {
        OrderLine {
            order_num,
            region: String::from("Porto da Serra"),
            item_name: String::from(item_name),
            quantity,
            ask_item_name: String::from("Minério"),
            ask_quantity: 4,
            mine,
            quality: None,
        }
    }

    #[test]
    fn market_panel_renders_offer_ask_region_and_mine_tag() {
        let orders = vec![order(7, "Madeira", 10, true)];
        let view = market_view(
            &MarketForm::default(),
            &orders,
            &KnownCatalog::default(),
            &[],
        );
        let (label, action) = &view.orders[0];
        let text = format!("{label} {action}");

        for expected in [
            "#7",
            "10 Madeira",
            "4 Minério",
            "Porto da Serra",
            "[MINHA]",
            "Cancelar",
        ] {
            assert!(text.contains(expected), "{text}");
        }
    }

    #[test]
    fn orders_sort_mine_first_then_by_item() {
        let orders = sorted_orders(vec![
            order(1, "Minério", 1, false),
            order(2, "Madeira", 1, true),
            order(3, "Madeira", 1, false),
        ]);

        assert_eq!(
            orders
                .iter()
                .map(|order| order.order_num)
                .collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
    }

    #[test]
    fn form_submission_triggers_barter_offer() {
        let (catalog, wood, ore) = two_items();
        let form = MarketForm {
            item_index: 0,
            quantity: String::from("12"),
            ask_index: 1,
            ask_quantity: String::from("5"),
            ..MarketForm::default()
        };

        assert_eq!(
            form_create_intent(&form, &catalog, &[]),
            Some(MarketIntent::Create(CreateSellOrder {
                item: wood,
                quantity: 12,
                ask_item: ore,
                ask_quantity: 5,
                instance: None,
            }))
        );
    }

    #[test]
    fn equipment_offer_names_the_exact_piece() {
        let (catalog, wood, _) = two_items();
        let (plain, rare) = (ItemInstanceId::new(), ItemInstanceId::new());
        let piece = |id, rarity| StorageLine {
            item: wood,
            item_name: String::from("Madeira"),
            quantity: 1,
            instance: Some(id),
            quality: Some(marvyr_domain_items::Quality {
                rarity,
                affixes: Vec::new(),
                gems: Vec::new(),
                map_mods: Vec::new(),
                aspect: None,
            }),
        };
        let storage = vec![
            piece(plain, marvyr_domain_items::Rarity::Normal),
            piece(rare, marvyr_domain_items::Rarity::Rare),
        ];
        let mut form = MarketForm {
            quantity: String::from("1"),
            ask_index: 1,
            ask_quantity: String::from("5"),
            ..MarketForm::default()
        };
        apply_market_button(
            &mut form,
            MarketButton::Piece(1),
            false,
            &catalog,
            &storage,
            &[],
        );
        let view = market_view(&form, &[], &catalog, &storage);
        assert!(view.piece.as_deref().is_some_and(|p| p.starts_with("Raro")));
        let Some(MarketIntent::Create(intent)) = form_create_intent(&form, &catalog, &storage)
        else {
            panic!("sem intent");
        };
        assert_eq!(intent.instance, Some(rare));
    }

    #[test]
    fn mine_row_cancels_and_other_row_accepts() {
        assert_eq!(
            order_intent(&order(7, "Madeira", 10, true)),
            MarketIntent::Cancel(CancelSellOrder { order_num: 7 })
        );
        assert_eq!(
            order_intent(&order(7, "Madeira", 10, false)),
            MarketIntent::Buy(BuySellOrder { order_num: 7 })
        );
    }

    #[test]
    fn step_buttons_adjust_fields_and_never_go_negative() {
        let catalog = KnownCatalog::default();
        let mut form = MarketForm::default();
        let step = |form: &mut MarketForm, field, delta, shift| {
            apply_market_button(
                form,
                MarketButton::Step(field, delta),
                shift,
                &catalog,
                &[],
                &[],
            )
        };

        assert_eq!(step(&mut form, FormFocus::Quantity, 1, false), None);
        assert_eq!(form.quantity, "1");
        assert_eq!(form.focus, FormFocus::Quantity);
        step(&mut form, FormFocus::AskQuantity, 1, true);
        assert_eq!(form.ask_quantity, "10");
        step(&mut form, FormFocus::AskQuantity, -1, true);
        step(&mut form, FormFocus::AskQuantity, -1, true);
        assert_eq!(form.ask_quantity, "0");
    }

    #[test]
    fn pick_buttons_choose_each_side_separately() {
        let (catalog, _, _) = two_items();
        let mut form = MarketForm::default();
        apply_market_button(
            &mut form,
            MarketButton::Pick(FormFocus::AskItem, 1),
            false,
            &catalog,
            &[],
            &[],
        );
        assert_eq!((form.item_index, form.ask_index), (0, 1));
        apply_market_button(
            &mut form,
            MarketButton::Pick(FormFocus::AskItem, 1),
            false,
            &catalog,
            &[],
            &[],
        );
        assert_eq!(form.ask_index, 1, "não passa do fim do catálogo");
    }

    #[test]
    fn order_buttons_select_or_execute_like_enter() {
        let orders = vec![order(3, "Madeira", 2, false), order(4, "Madeira", 1, true)];
        let catalog = KnownCatalog::default();
        let mut form = MarketForm {
            focus: FormFocus::AskQuantity,
            ..MarketForm::default()
        };

        let intent = apply_market_button(
            &mut form,
            MarketButton::SelectOrder(1),
            false,
            &catalog,
            &[],
            &orders,
        );
        assert_eq!(intent, None);
        assert_eq!((form.focus, form.selected_order), (FormFocus::Orders, 1));

        let intent = apply_market_button(
            &mut form,
            MarketButton::ExecuteOrder(1),
            false,
            &catalog,
            &[],
            &orders,
        );
        assert_eq!(
            intent,
            Some(MarketIntent::Cancel(CancelSellOrder { order_num: 4 }))
        );
    }
}
