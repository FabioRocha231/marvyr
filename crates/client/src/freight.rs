//! v52: frete entre jogadores no client — aba "Frete" do porto. O quadro
//! (`FreightBoard`) vem do servidor; aqui a lista com Levar/Cancelar e o
//! formulário de anunciar (carga, destino, prêmio, caução). Tudo o que
//! importa (escrow, prazos, entrega) é do servidor.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{
    AcceptFreight, CancelFreight, FreightBoard, FreightLine, PostFreight, StorageLine,
};
use marvyr_shared::ids::ItemDefinitionId;

use crate::i18n::{tr, trf};
use crate::net::ReliableChannel;
use crate::ui;

#[derive(Resource, Default)]
pub struct KnownFreight(pub Option<FreightBoard>);

/// Formulário de anúncio: índices nas listas do armazém/portos.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct FreightForm {
    pub cargo: usize,
    pub cargo_qty: u32,
    pub dest: usize,
    pub reward: usize,
    pub reward_qty: u32,
    pub collateral: u32,
}

impl Default for FreightForm {
    fn default() -> Self {
        Self {
            cargo: 0,
            cargo_qty: 10,
            dest: 0,
            reward: 0,
            reward_qty: 5,
            collateral: 10,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreightButton {
    Accept(u32),
    Cancel(u32),
    Cargo(i8),
    CargoQty(i32),
    Dest(i8),
    Reward(i8),
    RewardQty(i32),
    Collateral(i32),
    Post,
}

/// O que a aba mostra (comparável: só remonta quando muda).
#[derive(Debug, Clone, PartialEq)]
pub struct FreightView {
    pub lines: Vec<FreightLine>,
    pub cargo: Option<(String, u32)>,
    pub cargo_qty: u32,
    pub dest: Option<String>,
    pub reward: Option<(String, u32)>,
    pub reward_qty: u32,
    pub collateral: u32,
}

pub struct FreightPlugin;

impl Plugin for FreightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownFreight>()
            .init_resource::<FreightForm>()
            .add_systems(Update, (receive, handle_clicks).chain());
    }
}

fn receive(
    mut boards: EventReader<ClientReceiveMessage<FreightBoard>>,
    mut known: ResMut<KnownFreight>,
) {
    if let Some(board) = boards.read().last() {
        let board = board.message().clone();
        if known.0.as_ref() != Some(&board) {
            known.0 = Some(board);
        }
    }
}

/// Destinos possíveis: os portos menos o de agora.
fn destinations(board: Option<&FreightBoard>, here: &str) -> Vec<String> {
    board
        .map(|b| b.ports.iter().filter(|p| *p != here).cloned().collect())
        .unwrap_or_default()
}

fn pick<T: Clone>(list: &[T], index: usize) -> Option<T> {
    (!list.is_empty()).then(|| list[index % list.len()].clone())
}

pub fn freight_view(
    known: &KnownFreight,
    form: &FreightForm,
    storage: &[StorageLine],
    here: &str,
) -> FreightView {
    let goods: Vec<(String, u32)> = storage
        .iter()
        .map(|line| (line.item_name.clone(), line.quantity))
        .collect();
    FreightView {
        lines: known
            .0
            .as_ref()
            .map(|b| b.lines.clone())
            .unwrap_or_default(),
        cargo: pick(&goods, form.cargo),
        cargo_qty: form.cargo_qty,
        dest: pick(&destinations(known.0.as_ref(), here), form.dest),
        reward: pick(&goods, form.reward),
        reward_qty: form.reward_qty,
        collateral: form.collateral,
    }
}

fn button(parent: &mut ChildBuilder, label: impl Into<String>, action: FreightButton) {
    parent
        .spawn((ui::button(Node::default(), ui::BUTTON_BG), action))
        .with_children(|b| {
            b.spawn(ui::text(label, 13.0, ui::GOLD));
        });
}

fn stepper(parent: &mut ChildBuilder, label: String, minus: FreightButton, plus: FreightButton) {
    parent
        .spawn(Node {
            column_gap: Val::Px(6.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            button(row, "◀", minus);
            row.spawn(ui::text(label, 13.0, ui::TEXT));
            button(row, "▶", plus);
        });
}

pub fn spawn_freight_body(parent: &mut ChildBuilder, view: &FreightView) {
    parent.spawn(ui::text(
        tr("FRETE ENTRE CAPITÃES - leve a carga de outro capitão e ganhe o prêmio; a caução volta na entrega"),
        12.0,
        ui::PANEL_BORDER,
    ));
    if view.lines.is_empty() {
        parent.spawn(ui::text(
            tr("Nenhum frete neste porto."),
            13.0,
            ui::TEXT_DIM,
        ));
    }
    for line in &view.lines {
        parent
            .spawn(Node {
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn(ui::text(
                    trf(
                        "#{0} {1} → {2}: {3} {4} · prêmio {5} {6} · caução {7} · {8} min",
                        &[
                            &line.num.to_string(),
                            &tr(&line.origin),
                            &tr(&line.dest),
                            &line.cargo_qty.to_string(),
                            &tr(&line.cargo_item),
                            &line.reward_qty.to_string(),
                            &tr(&line.reward_item),
                            &line.collateral.to_string(),
                            &line.minutes_left.to_string(),
                        ],
                    ),
                    13.0,
                    if line.carrying {
                        ui::OK_GREEN
                    } else {
                        ui::TEXT
                    },
                ));
                if line.carrying {
                    row.spawn(ui::text(tr("no seu porão"), 12.0, ui::OK_GREEN));
                } else if line.mine && !line.in_transit {
                    button(row, tr("Cancelar"), FreightButton::Cancel(line.num));
                } else if line.mine {
                    row.spawn(ui::text(tr("a caminho"), 12.0, ui::TEXT_DIM));
                } else {
                    button(row, tr("Levar"), FreightButton::Accept(line.num));
                }
            });
    }
    parent.spawn(ui::text(tr("Anunciar frete daqui"), 14.0, ui::BRASS));
    let (Some((cargo, _)), Some(dest), Some((reward, _))) = (&view.cargo, &view.dest, &view.reward)
    else {
        parent.spawn(ui::text(
            tr("Guarde carga no armazém deste porto para anunciar."),
            13.0,
            ui::TEXT_DIM,
        ));
        return;
    };
    stepper(
        parent,
        trf("Carga: {0}", &[&tr(cargo)]),
        FreightButton::Cargo(-1),
        FreightButton::Cargo(1),
    );
    stepper(
        parent,
        trf("Quantidade: {0}", &[&view.cargo_qty.to_string()]),
        FreightButton::CargoQty(-5),
        FreightButton::CargoQty(5),
    );
    stepper(
        parent,
        trf("Destino: {0}", &[&tr(dest)]),
        FreightButton::Dest(-1),
        FreightButton::Dest(1),
    );
    stepper(
        parent,
        trf("Prêmio: {0}", &[&tr(reward)]),
        FreightButton::Reward(-1),
        FreightButton::Reward(1),
    );
    stepper(
        parent,
        trf("Quantidade do prêmio: {0}", &[&view.reward_qty.to_string()]),
        FreightButton::RewardQty(-1),
        FreightButton::RewardQty(1),
    );
    stepper(
        parent,
        trf(
            "Caução: {0} {1}",
            &[&view.collateral.to_string(), &tr(cargo)],
        ),
        FreightButton::Collateral(-5),
        FreightButton::Collateral(5),
    );
    button(parent, tr("Anunciar"), FreightButton::Post);
}

fn step(value: u32, delta: i32) -> u32 {
    (value as i64 + i64::from(delta)).clamp(1, 999) as u32
}

fn cycle(index: usize, delta: i8, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    ((index % len) as i64 + i64::from(delta)).rem_euclid(len as i64) as usize
}

fn handle_clicks(
    buttons: Query<(&Interaction, &FreightButton), Changed<Interaction>>,
    known: Res<KnownFreight>,
    storage: Res<crate::port_screen::KnownPortStorage>,
    port: Res<crate::port_screen::DockedPortName>,
    mut form: ResMut<FreightForm>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let goods: Vec<ItemDefinitionId> = storage.0.iter().map(|line| line.item).collect();
    let ports = destinations(known.0.as_ref(), &port.0);
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *button {
            FreightButton::Accept(num) => {
                let _ =
                    connection_manager.send_message::<ReliableChannel, _>(&AcceptFreight { num });
            }
            FreightButton::Cancel(num) => {
                let _ =
                    connection_manager.send_message::<ReliableChannel, _>(&CancelFreight { num });
            }
            FreightButton::Cargo(d) => form.cargo = cycle(form.cargo, d, goods.len()),
            FreightButton::CargoQty(d) => form.cargo_qty = step(form.cargo_qty, d),
            FreightButton::Dest(d) => form.dest = cycle(form.dest, d, ports.len()),
            FreightButton::Reward(d) => form.reward = cycle(form.reward, d, goods.len()),
            FreightButton::RewardQty(d) => form.reward_qty = step(form.reward_qty, d),
            FreightButton::Collateral(d) => form.collateral = step(form.collateral, d),
            FreightButton::Post => {
                let (Some(cargo), Some(reward), Some(dest)) = (
                    pick(&goods, form.cargo),
                    pick(&goods, form.reward),
                    pick(&ports, form.dest),
                ) else {
                    continue;
                };
                let _ = connection_manager.send_message::<ReliableChannel, _>(&PostFreight {
                    dest,
                    cargo_item: cargo,
                    cargo_qty: form.cargo_qty,
                    reward_item: reward,
                    reward_qty: form.reward_qty,
                    collateral: form.collateral,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steppers_cycle_and_clamp() {
        assert_eq!(cycle(0, -1, 3), 2);
        assert_eq!(cycle(2, 1, 3), 0);
        assert_eq!(cycle(5, 1, 0), 0);
        assert_eq!(step(3, -5), 1);
        assert_eq!(step(998, 5), 999);
    }

    #[test]
    fn destinations_leave_out_the_port_you_are_in() {
        let board = FreightBoard {
            lines: Vec::new(),
            ports: vec![
                String::from("Porto da Serra"),
                String::from("Porto da Mina"),
            ],
        };
        assert_eq!(
            destinations(Some(&board), "Porto da Serra"),
            vec![String::from("Porto da Mina")]
        );
    }
}
