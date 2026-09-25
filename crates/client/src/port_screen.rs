//! Tela de porto (MF-042, MF-058): modal bevy_ui quando atracado, com abas
//! de storage, loadout, crafting, shipyard e mercado. Teclado (Tab, setas,
//! Enter, ESC) e mouse (abas e linhas de ação clicáveis) disparam as mesmas
//! ações; sem snapshot de storage (gap documentado na aba Loadout).

use bevy::ecs::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_domain_crafting::recipe::StationKind;
use marvyr_domain_items::{
    EquipmentDefinition, EquipmentSlot, EquipmentStats, ItemDefinition, ItemKind,
};
use marvyr_domain_ships::{
    can_equip, cosmetic_by_code, CosmeticSlot, ShipDefinition, ShipKind, SlotSpec,
};
use marvyr_protocol::{
    CosmeticsSnapshot, CraftItem, CraftResult, DockResult, EquipItem, ItemLine, LoadoutLine,
    LoadoutResult, LoadoutSnapshot, MarketResult, PortStorageSnapshot, RecipeEntry,
    StorageDepositAll, StorageLine, StorageWithdrawAll, StoredElsewhere, Undock, UnequipItem,
    WearCosmetic,
};
use marvyr_shared::ids::{ItemDefinitionId, ShipDefinitionId};

use crate::crafting::KnownRecipes;
use crate::guild::{
    contracts_view, guild_view, spawn_contracts_body, spawn_guild_body, ContractFeedback,
    ContractsView, GuildPlugin, GuildView, KnownContracts, KnownGuildPrices,
};
use crate::i18n::{tr, trf, Lang};
use crate::market::{
    market_view, spawn_market_body, KnownCatalog, KnownOrders, MarketFeedback, MarketForm,
    MarketView,
};
use crate::net::{KnownShipKind, MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui::{self, UiButton};

/// Nome do porto atracado mais recente, extraído do `DockResult.reason`.
#[derive(Resource, Debug, Default)]
pub struct DockedPortName(pub String);

/// Último snapshot de loadout do servidor.
#[derive(Resource, Debug, Default)]
pub struct KnownLoadout(pub Vec<LoadoutLine>);

/// Último snapshot de storage do porto onde o jogador atracou, e o total
/// guardado em cada um dos outros portos (o armazém é por porto).
#[derive(Resource, Debug, Default)]
pub struct KnownPortStorage(pub Vec<StorageLine>, pub Vec<StoredElsewhere>);

/// Último veredito de loadout para a aba correspondente.
#[derive(Resource, Debug, Default)]
pub struct LoadoutFeedback(pub Option<LoadoutResult>);

/// Último veredito de craft para as abas Crafting/Shipyard.
#[derive(Resource, Debug, Default)]
pub struct CraftFeedback(pub Option<CraftResult>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortTab {
    Storage,
    Loadout,
    Crafting,
    Shipyard,
    Market,
    Guild,
    Contracts,
}

impl PortTab {
    pub const ALL: [PortTab; 7] = [
        PortTab::Storage,
        PortTab::Loadout,
        PortTab::Crafting,
        PortTab::Shipyard,
        PortTab::Market,
        PortTab::Guild,
        PortTab::Contracts,
    ];

    pub fn next(self) -> Self {
        match self {
            PortTab::Storage => PortTab::Loadout,
            PortTab::Loadout => PortTab::Crafting,
            PortTab::Crafting => PortTab::Shipyard,
            PortTab::Shipyard => PortTab::Market,
            PortTab::Market => PortTab::Guild,
            PortTab::Guild => PortTab::Contracts,
            PortTab::Contracts => PortTab::Storage,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            PortTab::Storage => PortTab::Contracts,
            PortTab::Contracts => PortTab::Guild,
            PortTab::Guild => PortTab::Market,
            PortTab::Loadout => PortTab::Storage,
            PortTab::Crafting => PortTab::Loadout,
            PortTab::Shipyard => PortTab::Crafting,
            PortTab::Market => PortTab::Shipyard,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PortTab::Storage => "Porão",
            PortTab::Loadout => "Equipamento",
            PortTab::Crafting => "Fabricação",
            PortTab::Shipyard => "Estaleiro",
            PortTab::Market => "Mercado",
            PortTab::Guild => "Guilda",
            PortTab::Contracts => "Contratos",
        }
    }
}

/// Estado local da tela: aba ativa e ação selecionada.
#[derive(Resource, Debug)]
pub struct PortScreenState {
    pub active_tab: PortTab,
    pub selected_action: usize,
}

impl Default for PortScreenState {
    fn default() -> Self {
        // Dev (§39): MARVYR_PORT_TAB=Guilda|Contratos abre direto na aba
        // (capturas MARVYR_SHOT sem teclado).
        let dev_tab = std::env::var("MARVYR_PORT_TAB").ok().and_then(|label| {
            PortTab::ALL
                .into_iter()
                .find(|tab| tab.label().eq_ignore_ascii_case(&label))
        });
        Self {
            active_tab: dev_tab.unwrap_or(PortTab::Storage),
            selected_action: 0,
        }
    }
}

/// Raiz da tela de porto (modal bevy_ui); visível apenas enquanto atracado.
#[derive(Component)]
pub struct PortScreen;

/// Área de conteúdo da aba ativa; reconstruída quando o `BodyView` muda.
#[derive(Component)]
struct PortBody;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum PortText {
    Title,
    Status,
}

#[derive(Component, Debug, Clone, Copy)]
struct TabButton(PortTab);

/// Linha de ação clicável: índice em `port_actions` da aba ativa.
#[derive(Component, Debug, Clone, Copy)]
struct PortActionButton(usize);

#[derive(Component)]
struct UndockButton;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PortAction {
    DepositAll,
    WithdrawAll,
    Unequip(EquipmentSlot),
    Equip(ItemDefinitionId, EquipmentSlot, String),
    Craft(u32),
    /// MV-066: vestir (`code` > 0) ou tirar (`code` 0) um cosmético.
    Wear {
        slot: u8,
        code: u8,
    },
    Undock,
}

/// O que o corpo da tela mostra; comparado a cada quadro para reconstruir.
#[derive(Debug, Clone, PartialEq)]
enum BodyView {
    Port {
        info: Vec<String>,
        actions: Vec<String>,
        selected: usize,
    },
    Market(MarketView),
    Guild(GuildView),
    Contracts(ContractsView),
}

/// Snapshots que alimentam as ações do porto.
#[derive(SystemParam)]
struct PortData<'w> {
    loadout: Res<'w, KnownLoadout>,
    storage: Res<'w, KnownPortStorage>,
    catalog: Res<'w, KnownCatalog>,
    ship_kind: Res<'w, KnownShipKind>,
    recipes: Res<'w, KnownRecipes>,
    cosmetics: Res<'w, crate::net::MyCosmetics>,
}

impl PortData<'_> {
    fn actions(&self, tab: PortTab) -> Vec<PortAction> {
        let mut actions = port_actions(
            tab,
            &self.loadout.0,
            &self.recipes.0,
            &self.storage.0,
            &self.catalog,
            self.ship_kind.0,
        );
        if let (PortTab::Loadout, Some(cosmetics)) = (tab, &self.cosmetics.0) {
            // Antes do "Desatracar", que fecha a lista.
            let undock = actions.pop();
            actions.extend(cosmetic_actions(cosmetics));
            actions.extend(undock);
        }
        actions
    }
}

/// Slot de rede do cosmético (0 vela, 1 bandeira).
fn wire_slot(slot: CosmeticSlot) -> u8 {
    match slot {
        CosmeticSlot::Sail => 0,
        CosmeticSlot::Flag => 1,
    }
}

/// Vestir o que o capitão possui e não está usando; tirar o que está.
fn cosmetic_actions(cosmetics: &CosmeticsSnapshot) -> Vec<PortAction> {
    let mut actions: Vec<PortAction> = cosmetics
        .owned
        .iter()
        .filter(|code| ![cosmetics.sail, cosmetics.flag].contains(code))
        .filter_map(|&code| {
            let cosmetic = cosmetic_by_code(code)?;
            Some(PortAction::Wear {
                slot: wire_slot(cosmetic.slot),
                code,
            })
        })
        .collect();
    for (slot, worn) in [(0, cosmetics.sail), (1, cosmetics.flag)] {
        if worn != 0 {
            actions.push(PortAction::Wear { slot, code: 0 });
        }
    }
    actions
}

/// Linha do visual atual (aba Equipamento).
fn cosmetic_line(cosmetics: &CosmeticsSnapshot) -> String {
    let name = |code: u8, default: &str| {
        tr(cosmetic_by_code(code).map_or(default, |cosmetic| cosmetic.name))
    };
    trf(
        "Visual: {0} · {1} — só aparência",
        &[
            &name(cosmetics.sail, "velas do casco"),
            &name(cosmetics.flag, "bandeira da casa"),
        ],
    )
}

#[derive(SystemParam)]
struct PortFeedback<'w> {
    loadout: Res<'w, LoadoutFeedback>,
    craft: Res<'w, CraftFeedback>,
    market: Res<'w, MarketFeedback>,
}

pub struct PortPlugin;

impl Plugin for PortPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DockedPortName>()
            .init_resource::<KnownLoadout>()
            .init_resource::<KnownPortStorage>()
            .init_resource::<PortScreenState>()
            .init_resource::<LoadoutFeedback>()
            .init_resource::<CraftFeedback>()
            .add_plugins(GuildPlugin)
            .add_systems(Startup, spawn_port_screen)
            .add_systems(
                Update,
                (
                    handle_loadout_snapshot,
                    handle_port_storage_snapshot,
                    handle_loadout_result,
                    handle_craft_result,
                    handle_dock_result,
                    toggle_port_screen,
                    handle_port_input,
                    handle_port_clicks,
                    update_port_screen,
                ),
            );
    }
}

fn handle_loadout_snapshot(
    mut events: EventReader<ClientReceiveMessage<LoadoutSnapshot>>,
    mut known: ResMut<KnownLoadout>,
) {
    for event in events.read() {
        known.0 = event.message().slots.clone();
    }
}

fn handle_port_storage_snapshot(
    mut events: EventReader<ClientReceiveMessage<PortStorageSnapshot>>,
    mut known: ResMut<KnownPortStorage>,
) {
    for event in events.read() {
        known.0 = event.message().lines.clone();
        known.1 = event.message().elsewhere.clone();
    }
}

fn ship_definition(kind: Option<ShipKind>, loadout: &[LoadoutLine]) -> Option<ShipDefinition> {
    let kind = kind?;
    Some(ShipDefinition {
        id: ShipDefinitionId::new(),
        kind,
        display_name: String::new(),
        slots: loadout
            .iter()
            .map(|line| SlotSpec {
                kind: line.slot,
                accepts_tag: None,
            })
            .collect(),
        cargo_capacity: 0,
        base_speed: 0.0,
        base_turn_rate: 0.0,
        base_hp: 0,
        base_weapon_damage: 0,
        base_weapon_range: 0.0,
    })
}

fn catalog_line(catalog: &KnownCatalog, item: ItemDefinitionId) -> Option<&ItemLine> {
    catalog.0.values().find(|line| line.id == item)
}

fn item_definition(line: &ItemLine) -> Option<ItemDefinition> {
    let slot = line.equipment_slot?;
    Some(ItemDefinition {
        id: line.id,
        kind: ItemKind::Equipment,
        equipment: Some(EquipmentDefinition {
            slot,
            stats: EquipmentStats::default(),
        }),
        max_stack: 1,
        base_weight: line.weight,
        tags: Default::default(),
        display_name: line.name.clone(),
    })
}

fn compatible_equip(
    storage: &[StorageLine],
    catalog: &KnownCatalog,
    loadout: &[LoadoutLine],
    kind: Option<ShipKind>,
) -> Vec<(EquipmentSlot, StorageLine)> {
    let Some(ship) = ship_definition(kind, loadout) else {
        return Vec::new();
    };
    storage
        .iter()
        .filter_map(|storage_line| {
            let item_line = catalog_line(catalog, storage_line.item)?;
            let item = item_definition(item_line)?;
            let slot = can_equip(&ship, &item).ok()?;
            Some((slot, storage_line.clone()))
        })
        .collect()
}

fn handle_loadout_result(
    mut events: EventReader<ClientReceiveMessage<LoadoutResult>>,
    mut feedback: ResMut<LoadoutFeedback>,
) {
    for event in events.read() {
        feedback.0 = Some(event.message().clone());
    }
}

fn handle_craft_result(
    mut events: EventReader<ClientReceiveMessage<CraftResult>>,
    mut feedback: ResMut<CraftFeedback>,
) {
    for event in events.read() {
        feedback.0 = Some(event.message().clone());
    }
}

fn port_name_from_reason(reason: &str) -> String {
    reason
        .strip_prefix("atracado em ")
        .unwrap_or(reason)
        .to_owned()
}

fn handle_dock_result(
    mut events: EventReader<ClientReceiveMessage<DockResult>>,
    mut port_name: ResMut<DockedPortName>,
) {
    for event in events.read() {
        let result = event.message();
        if result.success && result.docked {
            port_name.0 = port_name_from_reason(&result.reason);
        }
    }
}

/// Modal centrado (até 900x560): cabeçalho, abas, corpo e linha de status.
fn spawn_port_screen(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            GlobalZIndex(5),
            Visibility::Hidden,
            PortScreen,
        ))
        .with_children(|root| {
            root.spawn(ui::panel(Node {
                width: Val::Percent(92.0),
                max_width: Val::Px(900.0),
                height: Val::Percent(88.0),
                max_height: Val::Px(560.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(16.0)),
                ..default()
            }))
            .with_children(|panel| {
                panel
                    .spawn(Node {
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|header| {
                        header.spawn((ui::display("Porto", 28.0, ui::INK), PortText::Title));
                        header
                            .spawn(Node {
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(14.0),
                                ..default()
                            })
                            .with_children(|right| {
                                right
                                    .spawn((
                                        ui::button(Node::default(), ui::DANGER.with_alpha(0.35)),
                                        UndockButton,
                                    ))
                                    .with_children(|b| {
                                        b.spawn(crate::i18n::label("Desatracar [ESC]", 13.0, ui::TEXT));
                                    });
                            });
                    });
                panel
                    .spawn(Node {
                        column_gap: Val::Px(6.0),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    })
                    .with_children(|tabs| {
                        for tab in PortTab::ALL {
                            tabs.spawn((ui::button(Node::default(), ui::BUTTON_BG), TabButton(tab)))
                                .with_children(|b| {
                                    b.spawn(crate::i18n::label(tab.label(), 14.0, ui::TEXT));
                                });
                        }
                    });
                ui::double_rule(panel);
                panel.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        // Sem isto o flexbox não encolhe o corpo abaixo do
                        // conteúdo e a lista empurra os botões para fora.
                        min_height: Val::Px(0.0),
                        row_gap: Val::Px(4.0),
                        overflow: Overflow::clip_y(),
                        ..default()
                    },
                    PortBody,
                ));
                panel
                    .spawn(Node {
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|footer| {
                        footer.spawn((ui::text("", 13.0, ui::TEXT), PortText::Status));
                        footer.spawn(crate::i18n::label(
                            "Tab/Shift+Tab: abas · Setas: escolher · Enter: executar · ESC: desatracar",
                            11.0,
                            ui::TEXT_DIM,
                        ));
                    });
            });
        });
}

fn toggle_port_screen(
    docked: Res<MyDocked>,
    mut screens: Query<&mut Visibility, With<PortScreen>>,
) {
    let visibility = if docked.0 {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut entity in &mut screens {
        *entity = visibility;
    }
}

fn port_actions(
    tab: PortTab,
    loadout: &[LoadoutLine],
    recipes: &[RecipeEntry],
    storage: &[StorageLine],
    catalog: &KnownCatalog,
    ship_kind: Option<ShipKind>,
) -> Vec<PortAction> {
    let mut actions = match tab {
        PortTab::Storage => vec![PortAction::DepositAll, PortAction::WithdrawAll],
        PortTab::Loadout => {
            let mut actions: Vec<PortAction> = loadout
                .iter()
                .filter(|line| line.equipped)
                .map(|line| PortAction::Unequip(line.slot))
                .collect();
            actions.extend(
                compatible_equip(storage, catalog, loadout, ship_kind)
                    .into_iter()
                    .map(|(slot, line)| PortAction::Equip(line.item, slot, line.item_name.clone())),
            );
            actions
        }
        PortTab::Crafting => recipes_for_station(recipes, false)
            .into_iter()
            .map(|entry| PortAction::Craft(entry.recipe_id))
            .collect(),
        PortTab::Shipyard => recipes_for_station(recipes, true)
            .into_iter()
            .map(|entry| PortAction::Craft(entry.recipe_id))
            .collect(),
        PortTab::Market | PortTab::Guild | PortTab::Contracts => Vec::new(),
    };
    actions.push(PortAction::Undock);
    actions
}

fn recipes_for_station(recipes: &[RecipeEntry], dock: bool) -> Vec<&RecipeEntry> {
    recipes
        .iter()
        .filter(|entry| (entry.station == StationKind::Dock) == dock)
        .collect()
}

fn send_port_action(connection_manager: &mut ConnectionManager, action: &PortAction) {
    let _ =
        match action {
            PortAction::DepositAll => {
                connection_manager.send_message::<ReliableChannel, _>(&StorageDepositAll)
            }
            PortAction::WithdrawAll => {
                connection_manager.send_message::<ReliableChannel, _>(&StorageWithdrawAll)
            }
            PortAction::Unequip(slot) => {
                connection_manager.send_message::<ReliableChannel, _>(&UnequipItem { slot: *slot })
            }
            PortAction::Equip(_, _, _) => connection_manager
                .send_message::<ReliableChannel, _>(&equip_item_for(action).unwrap()),
            PortAction::Craft(recipe_id) => {
                connection_manager.send_message::<ReliableChannel, _>(&CraftItem {
                    recipe_id: *recipe_id,
                })
            }
            PortAction::Wear { slot, code } => connection_manager
                .send_message::<ReliableChannel, _>(&WearCosmetic {
                    slot: *slot,
                    code: *code,
                }),
            PortAction::Undock => connection_manager.send_message::<ReliableChannel, _>(&Undock),
        };
}

fn equip_item_for(action: &PortAction) -> Option<EquipItem> {
    match action {
        PortAction::Equip(item, _, _) => Some(EquipItem { item: *item }),
        _ => None,
    }
}

fn handle_port_input(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    mut state: ResMut<PortScreenState>,
    data: PortData,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if !docked.0 {
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&Undock);
        return;
    }

    if keys.just_pressed(KeyCode::Tab) {
        let backward = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        state.active_tab = if backward {
            state.active_tab.previous()
        } else {
            state.active_tab.next()
        };
        state.selected_action = 0;
        return;
    }

    // Abas de painel próprio (mouse): Mercado tem teclado em market.rs.
    if matches!(
        state.active_tab,
        PortTab::Market | PortTab::Guild | PortTab::Contracts
    ) {
        return;
    }

    let actions = data.actions(state.active_tab);
    if keys.just_pressed(KeyCode::ArrowUp) {
        state.selected_action = state.selected_action.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        state.selected_action = state
            .selected_action
            .saturating_add(1)
            .min(actions.len().saturating_sub(1));
    }
    if keys.just_pressed(KeyCode::Enter) {
        if let Some(action) = actions.get(state.selected_action) {
            send_port_action(&mut connection_manager, action);
        }
    }
}

/// Mouse: aba clicada vira ativa; linha de ação clicada = selecionar + Enter.
#[allow(clippy::type_complexity)]
fn handle_port_clicks(
    docked: Res<MyDocked>,
    mut state: ResMut<PortScreenState>,
    data: PortData,
    tabs: Query<(&Interaction, &TabButton), Changed<Interaction>>,
    rows: Query<(&Interaction, &PortActionButton), Changed<Interaction>>,
    undock: Query<&Interaction, (Changed<Interaction>, With<UndockButton>)>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if !docked.0 {
        return;
    }
    let pressed = |interaction: &Interaction| *interaction == Interaction::Pressed;
    if undock.iter().any(pressed) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&Undock);
        return;
    }
    if let Some((_, tab)) = tabs.iter().find(|(i, _)| pressed(i)) {
        state.active_tab = tab.0;
        state.selected_action = 0;
        return;
    }
    if let Some((_, row)) = rows.iter().find(|(i, _)| pressed(i)) {
        state.selected_action = row.0;
        if let Some(action) = data.actions(state.active_tab).get(row.0) {
            send_port_action(&mut connection_manager, action);
        }
    }
}

fn action_label(action: &PortAction, recipes: &[RecipeEntry]) -> String {
    match action {
        PortAction::DepositAll => tr("Depositar tudo"),
        PortAction::WithdrawAll => tr("Retirar tudo"),
        PortAction::Unequip(slot) => trf("Desequipar {0}", &[&tr(slot_label(*slot))]),
        PortAction::Equip(_, slot, item_name) => format!(
            "{}: {} [{}]",
            tr(slot_label(*slot)),
            tr(item_name),
            tr("Equipar")
        ),
        PortAction::Craft(recipe_id) => {
            let entry = recipes.iter().find(|entry| entry.recipe_id == *recipe_id);
            let verb = if entry.is_some_and(|entry| entry.station == StationKind::Dock) {
                "Construir {0}"
            } else {
                "Fabricar {0}"
            };
            let name = entry
                .map(|entry| entry.display_name.as_str())
                .unwrap_or("receita");
            trf(verb, &[&tr(name)])
        }
        PortAction::Wear { slot, code: 0 } => trf(
            "Tirar {0}",
            &[&tr(if *slot == 0 {
                "velas cosméticas"
            } else {
                "bandeira cosmética"
            })],
        ),
        PortAction::Wear { code, .. } => trf(
            "Usar {0}",
            &[&tr(
                cosmetic_by_code(*code).map_or("?", |cosmetic| cosmetic.name)
            )],
        ),
        PortAction::Undock => tr("Desatracar"),
    }
}

fn slot_label(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::Hull => "Casco",
        EquipmentSlot::Sail => "Velas",
        EquipmentSlot::Weapon => "Armas",
        EquipmentSlot::Aux => "Auxiliar",
    }
}

fn station_label(station: StationKind) -> &'static str {
    match station {
        StationKind::None => "Qualquer estação",
        StationKind::Workbench => "Bancada",
        StationKind::Anvil => "Bigorna",
        StationKind::Dock => "Doca",
    }
}

/// Receita da linha escolhida (se a linha for uma receita).
fn selected_recipe(actions: &[PortAction], selected: usize) -> Option<u32> {
    match actions.get(clamped_selection(actions, selected)) {
        Some(PortAction::Craft(recipe_id)) => Some(*recipe_id),
        _ => None,
    }
}

fn clamped_selection(actions: &[PortAction], selected: usize) -> usize {
    selected.min(actions.len().saturating_sub(1))
}

fn feedback_line(success: bool, reason: &str) -> String {
    format!(
        "{}: {}",
        tr(if success { "OK" } else { "ERRO" }),
        tr(reason)
    )
}

fn storage_lines(
    cargo_weight: Option<u32>,
    cargo_capacity: Option<u32>,
    storage: &[StorageLine],
) -> Vec<String> {
    let mut lines = vec![
        trf(
            "Porão: {0} / {1}",
            &[
                &cargo_weight.map_or_else(|| String::from("—"), |weight| weight.to_string()),
                &cargo_capacity.map_or_else(|| String::from("—"), |capacity| capacity.to_string()),
            ],
        ),
        tr("Armazém deste porto (cada porto guarda o seu):"),
    ];
    if storage.is_empty() {
        lines.push(format!("  {}", tr("(vazio)")));
    }
    lines.extend(
        storage
            .iter()
            .map(|line| format!("  {} x{}", tr(&line.item_name), line.quantity)),
    );
    lines
}

/// Onde ficou o resto: sem isso, depositar num porto e voltar a outro
/// parecia perda de item.
fn elsewhere_lines(elsewhere: &[StoredElsewhere]) -> Vec<String> {
    if elsewhere.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![tr("Guardado em outros portos:")];
    lines.extend(elsewhere.iter().map(|stored| {
        format!(
            "  {}",
            trf(
                "{0}: {1} itens",
                &[&tr(&stored.region), &stored.quantity.to_string()]
            )
        )
    }));
    lines
}

fn loadout_lines(
    loadout: &[LoadoutLine],
    storage: &[StorageLine],
    catalog: &KnownCatalog,
    ship_kind: Option<ShipKind>,
) -> Vec<String> {
    let mut lines = loadout
        .iter()
        .map(|line| {
            let name = if line.item_name.is_empty() {
                tr("(vazio)")
            } else {
                tr(&line.item_name)
            };
            format!("{}: {name}", tr(slot_label(line.slot)))
        })
        .collect::<Vec<_>>();
    if compatible_equip(storage, catalog, loadout, ship_kind).is_empty() {
        lines.push(tr("Armazém: nada compatível com este casco"));
    }
    lines
}

/// Só a receita escolhida mostra custo e estação: a lista inteira com os
/// ingredientes de tudo afogava quem só queria ver uma.
fn recipe_lines(recipes: &[RecipeEntry], dock: bool, selected: Option<u32>) -> Vec<String> {
    let mut lines = Vec::new();
    if dock {
        lines.push(tr("Receitas de casco — custos saem do armazém do porto."));
    }
    let chosen = recipes_for_station(recipes, dock)
        .into_iter()
        .find(|entry| Some(entry.recipe_id) == selected);
    if chosen.is_none() {
        lines.push(tr("Escolha uma receita para ver o custo."));
    }
    if let Some(entry) = chosen {
        let ingredients = entry
            .ingredients
            .iter()
            .map(|ingredient| format!("{}x {}", ingredient.quantity, tr(&ingredient.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let ingredients = if ingredients.is_empty() {
            String::from("—")
        } else {
            ingredients
        };
        // Uma linha por receita: a lista cabe no papel junto dos botões.
        let mut line = trf(
            "{0} · {1} · {2}",
            &[
                &tr(&entry.display_name),
                &tr(station_label(entry.station)),
                &ingredients,
            ],
        );
        // A saída só aparece quando diz algo além do nome da receita.
        if entry.output_quantity != 1 || entry.output_name != entry.display_name {
            line.push_str(&format!(
                " · {}",
                trf(
                    "rende {0} x{1}",
                    &[&tr(&entry.output_name), &entry.output_quantity.to_string()],
                )
            ));
        }
        lines.push(line);
    }
    lines
}

/// Linhas informativas da aba (as ações viram botões à parte).
#[allow(clippy::too_many_arguments)]
fn info_lines(
    tab: PortTab,
    cargo_weight: Option<u32>,
    cargo_capacity: Option<u32>,
    loadout: &[LoadoutLine],
    storage: &[StorageLine],
    catalog: &KnownCatalog,
    ship_kind: Option<ShipKind>,
    recipes: &[RecipeEntry],
    selected_recipe: Option<u32>,
) -> Vec<String> {
    match tab {
        PortTab::Storage => storage_lines(cargo_weight, cargo_capacity, storage),
        PortTab::Loadout => loadout_lines(loadout, storage, catalog, ship_kind),
        PortTab::Crafting => recipe_lines(recipes, false, selected_recipe),
        PortTab::Shipyard => recipe_lines(recipes, true, selected_recipe),
        PortTab::Market => vec![tr("Mercado regional")],
        PortTab::Guild | PortTab::Contracts => Vec::new(),
    }
}

/// Último veredito do servidor relevante para a aba: (sucesso, texto).
fn status_line(
    tab: PortTab,
    recipes: &[RecipeEntry],
    loadout_feedback: Option<&LoadoutResult>,
    craft_feedback: Option<&CraftResult>,
    market_feedback: Option<&MarketResult>,
) -> Option<(bool, String)> {
    match tab {
        PortTab::Storage | PortTab::Market | PortTab::Guild => {
            market_feedback.map(|r| (r.success, feedback_line(r.success, &r.reason)))
        }
        // Contratos usam `ContractFeedback` (ver update_port_screen).
        PortTab::Contracts => None,
        PortTab::Loadout => {
            loadout_feedback.map(|r| (r.success, feedback_line(r.success, &r.reason)))
        }
        PortTab::Crafting | PortTab::Shipyard => craft_feedback.map(|result| {
            let name = recipes
                .iter()
                .find(|entry| entry.recipe_id == result.recipe_id)
                .map(|entry| entry.display_name.as_str())
                .unwrap_or("receita");
            // Recusa diz o porquê (v16); sucesso só nomeia a receita.
            let detail = if result.success || result.reason.is_empty() {
                tr(name)
            } else {
                format!("{} · {}", tr(name), tr(&result.reason))
            };
            (result.success, feedback_line(result.success, &detail))
        }),
    }
}

/// Informação em cima (encolhe e corta quando a lista é longa, como as
/// receitas da Fabricação) e ações embaixo, sempre inteiras dentro do
/// papel. Com mais de quatro ações, elas vão em duas colunas.
fn spawn_port_body(
    parent: &mut ChildBuilder,
    info: &[String],
    actions: &[String],
    selected: usize,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_shrink: 1.0,
            min_height: Val::Px(0.0),
            row_gap: Val::Px(2.0),
            overflow: Overflow::clip_y(),
            ..default()
        })
        .with_children(|info_col| {
            for line in info {
                info_col.spawn(ui::text(line.as_str(), 14.0, ui::TEXT));
            }
        });
    let two_columns = actions.len() > 4;
    parent
        .spawn(Node {
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(8.0),
            row_gap: Val::Px(6.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|grid| {
            for (index, label) in actions.iter().enumerate() {
                let base = if index == selected {
                    ui::BUTTON_SELECTED
                } else {
                    ui::BUTTON_BG
                };
                grid.spawn((
                    ui::button(
                        Node {
                            justify_content: JustifyContent::Start,
                            width: if two_columns {
                                Val::Percent(49.0)
                            } else {
                                Val::Percent(100.0)
                            },
                            ..default()
                        },
                        base,
                    ),
                    PortActionButton(index),
                ))
                .with_children(|b| {
                    b.spawn(ui::text(label.as_str(), 14.0, ui::TEXT));
                });
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn update_port_screen(
    mut commands: Commands,
    state: Res<PortScreenState>,
    port_name: Res<DockedPortName>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    data: PortData,
    feedback: PortFeedback,
    market: (Res<MarketForm>, Res<KnownOrders>),
    guild: (
        Res<KnownGuildPrices>,
        Res<KnownContracts>,
        Res<ContractFeedback>,
        Res<Time>,
    ),
    bodies: Query<Entity, With<PortBody>>,
    mut texts: Query<(&mut Text, &mut TextColor, &PortText)>,
    mut tabs: Query<(&TabButton, &mut UiButton, &mut BackgroundColor)>,
    mut last_view: Local<Option<BodyView>>,
    lang: Res<Lang>,
) {
    // Troca de idioma: remonta o corpo com os textos novos.
    if lang.is_changed() {
        *last_view = None;
    }
    let tab = state.active_tab;
    let view = if tab == PortTab::Market {
        BodyView::Market(market_view(&market.0, &market.1 .0, &data.catalog))
    } else if tab == PortTab::Guild {
        BodyView::Guild(guild_view(
            guild.0 .0.as_ref(),
            &data.storage.0,
            &port_name.0,
        ))
    } else if tab == PortTab::Contracts {
        BodyView::Contracts(contracts_view(&guild.1, guild.3.elapsed_secs()))
    } else {
        let cargo = my_ship.0.and_then(|ship_id| {
            visuals
                .iter()
                .find(|visual| visual.target.ship_id == ship_id)
                .map(|visual| (visual.target.cargo_weight, visual.target.cargo_capacity))
        });
        let actions = data.actions(tab);
        let mut info = info_lines(
            tab,
            cargo.map(|(weight, _)| weight),
            cargo.map(|(_, capacity)| capacity),
            &data.loadout.0,
            &data.storage.0,
            &data.catalog,
            data.ship_kind.0,
            &data.recipes.0,
            selected_recipe(&actions, state.selected_action),
        );
        if let (PortTab::Loadout, Some(cosmetics)) = (tab, &data.cosmetics.0) {
            info.push(cosmetic_line(cosmetics));
        }
        if tab == PortTab::Storage {
            info.extend(elsewhere_lines(&data.storage.1));
        }
        BodyView::Port {
            info,
            actions: actions
                .iter()
                .map(|action| action_label(action, &data.recipes.0))
                .collect(),
            selected: clamped_selection(&actions, state.selected_action),
        }
    };
    if last_view.as_ref() != Some(&view) {
        for body in &bodies {
            commands
                .entity(body)
                .despawn_descendants()
                .with_children(|parent| match &view {
                    BodyView::Port {
                        info,
                        actions,
                        selected,
                    } => spawn_port_body(parent, info, actions, *selected),
                    BodyView::Market(market) => spawn_market_body(parent, market),
                    BodyView::Guild(guild) => spawn_guild_body(parent, guild),
                    BodyView::Contracts(contracts) => spawn_contracts_body(parent, contracts),
                });
        }
        *last_view = Some(view);
    }

    let status = if tab == PortTab::Contracts {
        guild
            .2
             .0
            .as_ref()
            .map(|r| (r.success, feedback_line(r.success, &r.reason)))
    } else {
        status_line(
            tab,
            &data.recipes.0,
            feedback.loadout.0.as_ref(),
            feedback.craft.0.as_ref(),
            feedback.market.0.as_ref(),
        )
    };
    for (mut text, mut color, kind) in &mut texts {
        let value = match kind {
            PortText::Title if port_name.0.is_empty() => tr("Porto: ?"),
            PortText::Title => tr(&port_name.0),
            PortText::Status => {
                color.0 = match &status {
                    Some((true, _)) => ui::OK_GREEN,
                    Some((false, _)) => ui::DANGER,
                    None => ui::TEXT_DIM,
                };
                status
                    .as_ref()
                    .map(|(_, line)| line.clone())
                    .unwrap_or_default()
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    for (button, mut style, mut bg) in &mut tabs {
        let base = if button.0 == tab {
            ui::BUTTON_SELECTED
        } else {
            ui::BUTTON_BG
        };
        if style.base != base {
            style.base = base;
            bg.0 = base;
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use marvyr_protocol::IngredientLine;

    use super::*;

    fn recipe(recipe_id: u32, station: StationKind) -> RecipeEntry {
        RecipeEntry {
            recipe_id,
            display_name: format!("Receita {recipe_id}"),
            station,
            ship_build: station == StationKind::Dock,
            output_name: format!("Saída {recipe_id}"),
            output_quantity: 1,
            ingredients: vec![IngredientLine {
                name: String::from("Madeira"),
                quantity: 5,
            }],
        }
    }

    fn loadout() -> KnownLoadout {
        KnownLoadout(vec![
            LoadoutLine {
                slot: EquipmentSlot::Hull,
                item_name: String::from("Casco Reforçado"),
                equipped: true,
            },
            LoadoutLine {
                slot: EquipmentSlot::Sail,
                item_name: String::new(),
                equipped: false,
            },
            LoadoutLine {
                slot: EquipmentSlot::Weapon,
                item_name: String::from("Canhão de Bronze"),
                equipped: true,
            },
            LoadoutLine {
                slot: EquipmentSlot::Aux,
                item_name: String::new(),
                equipped: false,
            },
        ])
    }

    fn storage_line(id: ItemDefinitionId, item_name: &str, quantity: u32) -> StorageLine {
        StorageLine {
            item: id,
            item_name: String::from(item_name),
            quantity,
        }
    }

    fn item_line(id: ItemDefinitionId, item_name: &str, slot: Option<EquipmentSlot>) -> ItemLine {
        ItemLine {
            id,
            name: String::from(item_name),
            weight: 5,
            equipment_slot: slot,
        }
    }

    /// Tudo o que a aba mostra (info + rótulos de ação + status) como texto.
    #[allow(clippy::too_many_arguments)]
    fn port_screen_text(
        _port_name: &str,
        state: &PortScreenState,
        cargo_weight: Option<u32>,
        cargo_capacity: Option<u32>,
        loadout: &[LoadoutLine],
        storage: &[StorageLine],
        catalog: &KnownCatalog,
        ship_kind: Option<ShipKind>,
        recipes: &[RecipeEntry],
        loadout_feedback: Option<&LoadoutResult>,
        craft_feedback: Option<&CraftResult>,
        market_feedback: Option<&MarketResult>,
    ) -> String {
        let tab = state.active_tab;
        let actions = port_actions(tab, loadout, recipes, storage, catalog, ship_kind);
        let mut lines = info_lines(
            tab,
            cargo_weight,
            cargo_capacity,
            loadout,
            storage,
            catalog,
            ship_kind,
            recipes,
            selected_recipe(&actions, state.selected_action),
        );
        lines.extend(
            port_actions(tab, loadout, recipes, storage, catalog, ship_kind)
                .iter()
                .map(|action| action_label(action, recipes)),
        );
        lines.extend(
            status_line(
                tab,
                recipes,
                loadout_feedback,
                craft_feedback,
                market_feedback,
            )
            .map(|(_, line)| line),
        );
        lines.join("\n")
    }

    #[test]
    fn port_screen_visibility_follows_docked_state() {
        let mut world = World::new();
        world.insert_resource(MyDocked(true));
        let entity = world.spawn((PortScreen, Visibility::Hidden)).id();

        world.run_system_once(toggle_port_screen).unwrap();
        assert_eq!(
            *world.get::<Visibility>(entity).unwrap(),
            Visibility::Visible
        );

        world.insert_resource(MyDocked(false));
        world.run_system_once(toggle_port_screen).unwrap();
        assert_eq!(
            *world.get::<Visibility>(entity).unwrap(),
            Visibility::Hidden
        );
    }

    #[test]
    fn tab_cycles_forward_and_backward() {
        let mut tab = PortTab::Storage;
        for expected in [
            PortTab::Loadout,
            PortTab::Crafting,
            PortTab::Shipyard,
            PortTab::Market,
            PortTab::Guild,
            PortTab::Contracts,
            PortTab::Storage,
        ] {
            tab = tab.next();
            assert_eq!(tab, expected);
        }

        let mut tab = PortTab::Storage;
        for expected in [
            PortTab::Contracts,
            PortTab::Guild,
            PortTab::Market,
            PortTab::Shipyard,
            PortTab::Crafting,
            PortTab::Loadout,
            PortTab::Storage,
        ] {
            tab = tab.previous();
            assert_eq!(tab, expected);
        }
    }

    #[test]
    fn storage_actions_send_deposit_and_withdraw() {
        let actions = port_actions(
            PortTab::Storage,
            &[],
            &[],
            &[],
            &KnownCatalog::default(),
            None,
        );

        assert_eq!(actions[0], PortAction::DepositAll);
        assert_eq!(actions[1], PortAction::WithdrawAll);
    }

    /// O Trevor depositou minério e achou que tinha sumido: a aba escondia
    /// o armazém. Agora mostra o que está guardado neste porto.
    #[test]
    fn storage_tab_lists_what_is_stored_here() {
        let ore = StorageLine {
            item: marvyr_shared::ids::ItemDefinitionId::new(),
            item_name: String::from("Minério"),
            quantity: 100,
        };
        let text = port_screen_text(
            "Porto da Mina",
            &PortScreenState {
                active_tab: PortTab::Storage,
                selected_action: 0,
            },
            Some(0),
            Some(100),
            &[],
            &[ore],
            &KnownCatalog::default(),
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(text.contains("Minério x100"), "{text}");
        assert!(elsewhere_lines(&[StoredElsewhere {
            region: String::from("Porto da Mina"),
            quantity: 100,
        }])
        .iter()
        .any(|line| line.contains("Porto da Mina") && line.contains("100")),);
    }

    #[test]
    fn storage_tab_shows_cargo_weight_and_capacity() {
        let text = port_screen_text(
            "Porto da Serra",
            &PortScreenState {
                active_tab: PortTab::Storage,
                selected_action: 0,
            },
            Some(8),
            Some(100),
            &[],
            &[],
            &KnownCatalog::default(),
            None,
            &[],
            None,
            None,
            None,
        );

        assert!(text.contains("Porão: 8 / 100"), "{text}");
    }

    #[test]
    fn loadout_lists_slots_and_only_unequips_equipped_slots() {
        let loadout = loadout();
        let text = port_screen_text(
            "Porto da Serra",
            &PortScreenState {
                active_tab: PortTab::Loadout,
                selected_action: 0,
            },
            Some(8),
            Some(100),
            &loadout.0,
            &[],
            &KnownCatalog::default(),
            Some(ShipKind::SmallMerchant),
            &[],
            None,
            None,
            None,
        );
        for expected in [
            "Casco: Casco Reforçado",
            "Velas: (vazio)",
            "Armas: Canhão de Bronze",
            "Auxiliar: (vazio)",
            "Armazém: nada compatível com este casco",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(!text.contains("use T/Y/U (debug)"), "{text}");

        let actions = port_actions(
            PortTab::Loadout,
            &loadout.0,
            &[],
            &[],
            &KnownCatalog::default(),
            Some(ShipKind::SmallMerchant),
        );
        assert_eq!(actions[0], PortAction::Unequip(EquipmentSlot::Hull));
        assert_eq!(actions[1], PortAction::Unequip(EquipmentSlot::Weapon));
        assert!(!actions
            .iter()
            .any(|action| matches!(action, PortAction::Craft(_))));
    }

    #[test]
    fn crafting_tab_lists_non_dock_recipes() {
        let recipes = vec![
            recipe(1, StationKind::Workbench),
            recipe(2, StationKind::Dock),
            recipe(3, StationKind::None),
            recipe(4, StationKind::Anvil),
        ];
        let text = port_screen_text(
            "Porto da Serra",
            &PortScreenState {
                active_tab: PortTab::Crafting,
                selected_action: 0,
            },
            None,
            None,
            &[],
            &[],
            &KnownCatalog::default(),
            None,
            &recipes,
            None,
            None,
            None,
        );
        assert!(text.contains("Receita 1"), "{text}");
        assert!(text.contains("Receita 3"), "{text}");
        assert!(text.contains("Receita 4"), "{text}");
        assert!(!text.contains("Receita 2"), "{text}");

        let entries = recipes_for_station(&recipes, false);
        assert_eq!(entries.len(), 3);
        let actions = port_actions(
            PortTab::Crafting,
            &[],
            &recipes,
            &[],
            &KnownCatalog::default(),
            None,
        );
        assert_eq!(actions[0], PortAction::Craft(1));
        assert_eq!(actions[1], PortAction::Craft(3));
    }

    #[test]
    fn shipyard_tab_lists_dock_recipes_only() {
        let recipes = vec![
            recipe(1, StationKind::Workbench),
            recipe(2, StationKind::Dock),
        ];
        let text = port_screen_text(
            "Porto da Serra",
            &PortScreenState {
                active_tab: PortTab::Shipyard,
                selected_action: 0,
            },
            None,
            None,
            &[],
            &[],
            &KnownCatalog::default(),
            None,
            &recipes,
            None,
            None,
            None,
        );
        assert!(text.contains("Receita 2"), "{text}");
        assert!(!text.contains("Receita 1"), "{text}");
        assert!(
            text.contains("Receitas de casco — custos saem do armazém do porto."),
            "{text}"
        );

        let entries = recipes_for_station(&recipes, true);
        assert_eq!(entries.len(), 1);
        let actions = port_actions(
            PortTab::Shipyard,
            &[],
            &recipes,
            &[],
            &KnownCatalog::default(),
            None,
        );
        assert_eq!(actions[0], PortAction::Craft(2));
    }

    #[test]
    fn craft_action_sends_craft_item_for_recipe_id() {
        let recipes = vec![recipe(7, StationKind::Workbench)];
        let actions = port_actions(
            PortTab::Crafting,
            &[],
            &recipes,
            &[],
            &KnownCatalog::default(),
            None,
        );

        assert_eq!(actions[0], PortAction::Craft(7));
    }

    #[test]
    fn market_tab_renders_market_body_inside_port_screen() {
        let mut world = World::new();
        world.insert_resource(PortScreenState {
            active_tab: PortTab::Market,
            selected_action: 0,
        });
        world.init_resource::<DockedPortName>();
        world.init_resource::<MyShip>();
        world.init_resource::<KnownLoadout>();
        world.init_resource::<KnownPortStorage>();
        world.init_resource::<KnownCatalog>();
        world.init_resource::<KnownShipKind>();
        world.init_resource::<KnownRecipes>();
        world.init_resource::<crate::net::MyCosmetics>();
        world.init_resource::<LoadoutFeedback>();
        world.init_resource::<CraftFeedback>();
        world.init_resource::<MarketFeedback>();
        world.init_resource::<MarketForm>();
        world.init_resource::<KnownOrders>();
        world.init_resource::<KnownGuildPrices>();
        world.init_resource::<KnownContracts>();
        world.init_resource::<ContractFeedback>();
        world.init_resource::<Time>();
        world.init_resource::<Lang>();
        world.run_system_once(spawn_port_screen).unwrap();
        let mut schedule = bevy::ecs::schedule::Schedule::default();
        schedule.add_systems(update_port_screen);

        schedule.run(&mut world);
        let mut market = world.query::<&crate::market::MarketButton>();
        assert!(market.iter(&world).count() > 0);

        world.insert_resource(PortScreenState::default());
        schedule.run(&mut world);
        assert_eq!(market.iter(&world).count(), 0);
        let mut rows = world.query::<&PortActionButton>();
        // Depositar, Retirar, Desatracar.
        assert_eq!(rows.iter(&world).count(), 3);
    }

    #[test]
    fn failed_market_result_surfaces_reason_in_status_line() {
        let feedback = MarketResult {
            success: false,
            reason: String::from("atraca primeiro (E)"),
        };

        let status = status_line(PortTab::Market, &[], None, None, Some(&feedback));
        assert_eq!(
            status,
            Some((false, String::from("ERRO: atraca primeiro (E)")))
        );
    }

    #[test]
    fn failed_craft_result_surfaces_reason_in_status_line() {
        let feedback = CraftResult {
            recipe_id: 99,
            success: false,
            reason: String::from("Armazém vazio neste porto: deposite materiais com Z."),
        };

        let status = status_line(PortTab::Crafting, &[], None, Some(&feedback), None);
        assert_eq!(
            status,
            Some((
                false,
                String::from(
                    "ERRO: receita · Armazém vazio neste porto: deposite materiais com Z."
                )
            ))
        );
    }

    #[test]
    fn undock_action_sends_undock() {
        let actions = port_actions(
            PortTab::Market,
            &[],
            &[],
            &[],
            &KnownCatalog::default(),
            None,
        );

        assert_eq!(actions[0], PortAction::Undock);
    }

    #[test]
    fn loadout_renders_equip_button_for_matching_storage_items() {
        let hull = ItemDefinitionId::new();
        let storage = vec![storage_line(hull, "Casco Reforçado", 1)];
        let catalog = KnownCatalog(std::collections::HashMap::from([(
            String::from("Casco Reforçado"),
            item_line(hull, "Casco Reforçado", Some(EquipmentSlot::Hull)),
        )]));
        let text = port_screen_text(
            "Porto da Serra",
            &PortScreenState {
                active_tab: PortTab::Loadout,
                selected_action: 0,
            },
            Some(8),
            Some(100),
            &loadout().0,
            &storage,
            &catalog,
            Some(ShipKind::SmallMerchant),
            &[],
            None,
            None,
            None,
        );

        assert!(text.contains("Casco: Casco Reforçado [Equipar]"), "{text}");
        assert!(!text.contains("use T/Y/U (debug)"), "{text}");
    }

    #[test]
    fn equip_button_builds_equip_item_intent() {
        let hull = ItemDefinitionId::new();
        let storage = vec![storage_line(hull, "Casco Reforçado", 1)];
        let catalog = KnownCatalog(std::collections::HashMap::from([(
            String::from("Casco Reforçado"),
            item_line(hull, "Casco Reforçado", Some(EquipmentSlot::Hull)),
        )]));
        let actions = port_actions(
            PortTab::Loadout,
            &loadout().0,
            &[],
            &storage,
            &catalog,
            Some(ShipKind::SmallMerchant),
        );

        let equip = actions
            .iter()
            .find_map(|action| match action {
                PortAction::Equip(item, slot, name) => Some((*item, *slot, name.as_str())),
                _ => None,
            })
            .expect("storage compatível gera ação Equipar");
        assert_eq!(equip.0, hull);
        assert_eq!(equip.1, EquipmentSlot::Hull);
        assert_eq!(equip.2, "Casco Reforçado");
        assert_eq!(
            equip_item_for(&PortAction::Equip(
                hull,
                EquipmentSlot::Hull,
                String::from("Casco Reforçado")
            )),
            Some(EquipItem { item: hull })
        );
    }

    #[test]
    fn items_missing_from_port_storage_do_not_spawn_equip_ui() {
        let actions = port_actions(
            PortTab::Loadout,
            &loadout().0,
            &[],
            &[],
            &KnownCatalog::default(),
            Some(ShipKind::SmallMerchant),
        );

        assert!(!actions
            .iter()
            .any(|action| matches!(action, PortAction::Equip(_, _, _))));
    }

    #[test]
    fn cosmetic_actions_offer_what_is_owned_and_remove_what_is_worn() {
        let code = |id| marvyr_domain_ships::cosmetic_code(id).unwrap();
        let (gold, azure, linen) = (code("sail-gold"), code("sail-azure"), code("flag-linen"));
        let cosmetics = CosmeticsSnapshot {
            owned: vec![gold, azure, linen],
            sail: gold,
            flag: 0,
        };
        assert_eq!(
            cosmetic_actions(&cosmetics),
            vec![
                PortAction::Wear {
                    slot: 0,
                    code: azure
                },
                PortAction::Wear {
                    slot: 1,
                    code: linen
                },
                PortAction::Wear { slot: 0, code: 0 },
            ]
        );
        assert!(cosmetic_line(&cosmetics).contains("Velas de Ouro"));
        assert!(cosmetic_actions(&CosmeticsSnapshot {
            owned: Vec::new(),
            sail: 0,
            flag: 0
        })
        .is_empty());
    }
}
