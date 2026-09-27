//! v62: companhia e guerra de território no client. Aba "Companhia" do
//! porto (fundar digitando o nome, aceitar convite, convidar quem está
//! atracado aqui, sair, frentes de guerra) e, no mar, o placar da guerra do
//! porto mais perto com a janela aberta. Tudo que vale (custo, membros,
//! influência, Senhor) é do servidor.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{
    CompanyAnswer, CompanyInvite, CompanyUpdate, CreateCompany, LeaveCompany, WarFront,
};

use crate::i18n::{tr, trf};
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::port_screen::{PortScreenState, PortTab};
use crate::ship::ShipVisual;
use crate::ui;

/// Até esta distância de um porto em guerra o placar aparece no mar (m).
const WAR_HUD_RANGE: f32 = 900.0;
const NAME_MAX: usize = 20;

#[derive(Resource, Default)]
pub struct KnownCompany(pub CompanyUpdate);

/// Nome sendo digitado para fundar.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct CompanyForm {
    pub typing: bool,
    pub name: String,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyButton {
    Type,
    Found,
    Accept,
    Decline,
    Invite(u32),
    Leave,
    /// v63: expulsar o membro da posição (o nome confirma).
    Kick(u16),
}

/// O que a aba mostra (comparável: só remonta quando muda).
#[derive(Debug, Clone, PartialEq)]
pub struct CompanyView {
    pub company: CompanyUpdate,
    pub form: CompanyForm,
}

#[derive(Component)]
struct WarPanel;

#[derive(Component)]
struct WarText;

pub struct CompanyPlugin;

impl Plugin for CompanyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownCompany>()
            .init_resource::<CompanyForm>()
            .add_systems(Startup, spawn_war_panel)
            .add_systems(
                Update,
                (
                    receive,
                    handle_clicks,
                    handle_typing,
                    auto_company,
                    update_war_panel,
                )
                    .chain(),
            );
    }
}

fn receive(
    mut updates: EventReader<ClientReceiveMessage<CompanyUpdate>>,
    mut known: ResMut<KnownCompany>,
) {
    if let Some(update) = updates.read().last() {
        let update = update.message().clone();
        if known.0 != update {
            known.0 = update;
        }
    }
}

pub fn company_view(known: &KnownCompany, form: &CompanyForm) -> CompanyView {
    CompanyView {
        company: known.0.clone(),
        form: form.clone(),
    }
}

fn button(parent: &mut ChildBuilder, label: impl Into<String>, action: CompanyButton) {
    parent
        .spawn((ui::button(Node::default(), ui::BUTTON_BG), action))
        .with_children(|b| {
            b.spawn(ui::text(label, 13.0, ui::GOLD));
        });
}

fn clock_label(secs: f32) -> String {
    let secs = secs.max(0.0) as u64;
    if secs >= 3600 {
        format!("{}h{:02}", secs / 3600, secs % 3600 / 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn member_label(member: &marvyr_protocol::CompanyMember) -> String {
    // A fonte não tem ★/●: marca em palavra.
    let mut label = member.name.clone();
    if member.leader {
        label.push_str(&format!(" ({})", tr("líder")));
    }
    if member.online {
        label.push_str(&format!(" ({})", tr("online")));
    }
    label
}

fn front_line(front: &WarFront) -> String {
    let holder = if front.holder.is_empty() {
        tr("ninguém")
    } else {
        front.holder.clone()
    };
    let when = if front.open {
        trf("GUERRA ABERTA, fecha em {0}", &[&clock_label(front.secs)])
    } else {
        trf("guerra em {0}", &[&clock_label(front.secs)])
    };
    trf(
        "{0}: manda {1} · {2} · você {3} × {4} rival",
        &[
            &tr(&front.port),
            &holder,
            &when,
            &front.mine.to_string(),
            &front.rival.to_string(),
        ],
    )
}

pub fn spawn_company_body(parent: &mut ChildBuilder, view: &CompanyView) {
    let company = &view.company;
    parent.spawn(ui::text(
        tr("COMPANHIA - aliados no mar; a influência de todos soma na guerra pelos portos"),
        12.0,
        ui::PANEL_BORDER,
    ));
    if let Some(from) = &company.invite_from {
        parent
            .spawn(Node {
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn(ui::text(
                    trf("Convite da companhia {0}", &[from]),
                    14.0,
                    ui::BRASS,
                ));
                button(row, tr("Aceitar"), CompanyButton::Accept);
                button(row, tr("Recusar"), CompanyButton::Decline);
            });
    }
    if company.name.is_empty() {
        parent.spawn(ui::text(
            tr("Fundar custa 40 Madeira e 40 Minério do armazém deste porto."),
            13.0,
            ui::TEXT,
        ));
        let shown = if view.form.typing {
            format!("{}_", view.form.name)
        } else if view.form.name.is_empty() {
            tr("(clique e digite o nome)")
        } else {
            view.form.name.clone()
        };
        parent
            .spawn(Node {
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn(ui::text(trf("Nome: {0}", &[&shown]), 14.0, ui::INK));
                button(row, tr("Digitar"), CompanyButton::Type);
                button(row, tr("Fundar"), CompanyButton::Found);
            });
        if view.form.typing {
            parent.spawn(ui::text(
                tr("Enter funda · Esc cancela · a tag sai das iniciais"),
                12.0,
                ui::TEXT_DIM,
            ));
        }
    } else {
        parent.spawn(ui::text(
            format!("[{}] {}", company.tag, company.name),
            18.0,
            ui::BRASS,
        ));
        let ports = if company.ports.is_empty() {
            tr("nenhum")
        } else {
            company
                .ports
                .iter()
                .map(|port| tr(port))
                .collect::<Vec<_>>()
                .join(", ")
        };
        parent.spawn(ui::text(
            trf("Portos da companhia: {0}", &[&ports]),
            13.0,
            ui::TEXT,
        ));
        if company.leader && company.members.iter().any(|member| !member.leader) {
            parent.spawn(ui::text(tr("Membros"), 14.0, ui::BRASS));
            for (index, member) in company.members.iter().enumerate() {
                parent
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(ui::text(member_label(member), 13.0, ui::TEXT));
                        if !member.leader {
                            button(row, tr("Expulsar"), CompanyButton::Kick(index as u16));
                        }
                    });
            }
        }
        let members = company
            .members
            .iter()
            .map(member_label)
            .collect::<Vec<_>>()
            .join("  ·  ");
        parent.spawn(ui::text(
            trf(
                "Membros ({0}): {1}",
                &[&company.members.len().to_string(), &members],
            ),
            13.0,
            ui::TEXT,
        ));
        if company.leader && !company.docked_here.is_empty() {
            parent.spawn(ui::text(tr("Capitães neste porto"), 14.0, ui::BRASS));
            for (ship_id, name) in &company.docked_here {
                parent
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(ui::text(name.clone(), 13.0, ui::TEXT));
                        button(row, tr("Convidar"), CompanyButton::Invite(*ship_id));
                    });
            }
        }
        button(parent, tr("Sair da companhia"), CompanyButton::Leave);
    }
    parent.spawn(ui::text(tr("Guerra pelos portos"), 14.0, ui::BRASS));
    for front in &company.wars {
        parent.spawn(ui::text(
            front_line(front),
            13.0,
            if front.open { ui::DANGER } else { ui::TEXT },
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_clicks(
    buttons: Query<(&Interaction, &CompanyButton), Changed<Interaction>>,
    known: Res<KnownCompany>,
    mut form: ResMut<CompanyForm>,
    mut modal: ResMut<crate::input::ModalOpen>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *button {
            CompanyButton::Type => {
                form.typing = true;
                modal.company = true;
            }
            CompanyButton::Found => {
                if !form.name.trim().is_empty() {
                    let _ = connection_manager.send_message::<ReliableChannel, _>(&CreateCompany {
                        name: form.name.trim().to_owned(),
                    });
                }
                form.typing = false;
                modal.company = false;
            }
            CompanyButton::Accept => {
                let _ = connection_manager
                    .send_message::<ReliableChannel, _>(&CompanyAnswer { accept: true });
            }
            CompanyButton::Decline => {
                let _ = connection_manager
                    .send_message::<ReliableChannel, _>(&CompanyAnswer { accept: false });
            }
            CompanyButton::Invite(target_ship_id) => {
                let _ = connection_manager
                    .send_message::<ReliableChannel, _>(&CompanyInvite { target_ship_id });
            }
            CompanyButton::Leave => {
                let _ = connection_manager.send_message::<ReliableChannel, _>(&LeaveCompany);
            }
            CompanyButton::Kick(index) => {
                if let Some(member) = known.0.members.get(usize::from(index)) {
                    let _ = connection_manager.send_message::<ReliableChannel, _>(
                        &marvyr_protocol::KickMember {
                            index,
                            name: member.name.clone(),
                        },
                    );
                }
            }
        }
    }
}

/// Digitação do nome: o modal segura o resto do jogo (E, Tab, números…).
fn handle_typing(
    mut keyboard: EventReader<KeyboardInput>,
    docked: Res<MyDocked>,
    state: Res<PortScreenState>,
    mut form: ResMut<CompanyForm>,
    mut modal: ResMut<crate::input::ModalOpen>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    // Saiu da aba ou do porto: para de digitar.
    if form.typing && (!docked.0 || state.active_tab != PortTab::Company) {
        form.typing = false;
        modal.company = false;
    }
    if !form.typing {
        return;
    }
    for event in keyboard.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        match &event.logical_key {
            Key::Enter => {
                if !form.name.trim().is_empty() {
                    let _ = connection_manager.send_message::<ReliableChannel, _>(&CreateCompany {
                        name: form.name.trim().to_owned(),
                    });
                }
                form.typing = false;
                modal.company = false;
            }
            Key::Escape => {
                form.typing = false;
                modal.company = false;
            }
            Key::Backspace => {
                form.name.pop();
            }
            Key::Space if form.name.chars().count() < NAME_MAX => form.name.push(' '),
            Key::Character(text) => {
                for c in text.chars() {
                    if (c.is_alphabetic() || c == '\'') && form.name.chars().count() < NAME_MAX {
                        form.name.push(c);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Dev (teste ao vivo sem teclado): MARVYR_AUTOCOMPANY=<nome> funda a
/// companhia atracado e convida quem estiver no porto; `=join` só aceita
/// convite. Tenta a cada 3 s.
fn auto_company(
    time: Res<Time>,
    mut clock: Local<f32>,
    known: Res<KnownCompany>,
    docked: Res<MyDocked>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let Ok(wanted) = std::env::var("MARVYR_AUTOCOMPANY") else {
        return;
    };
    *clock += time.delta_secs();
    if *clock < 3.0 || !docked.0 {
        return;
    }
    *clock = 0.0;
    let company = &known.0;
    if company.invite_from.is_some() && company.name.is_empty() {
        let _ =
            connection_manager.send_message::<ReliableChannel, _>(&CompanyAnswer { accept: true });
    } else if company.name.is_empty() && wanted != "join" {
        let _ =
            connection_manager.send_message::<ReliableChannel, _>(&CreateCompany { name: wanted });
    } else if let Some((target_ship_id, _)) = company.docked_here.first() {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&CompanyInvite {
            target_ship_id: *target_ship_id,
        });
    }
}

fn spawn_war_panel(mut commands: Commands) {
    commands
        .spawn((
            ui::panel(Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Px(84.0),
                margin: UiRect::left(Val::Px(-200.0)),
                width: Val::Px(400.0),
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                display: Display::None,
                ..default()
            }),
            crate::hud::SeaHud,
            WarPanel,
        ))
        .with_children(|panel| {
            panel.spawn((ui::face("", ui::FONT_BOLD, 13.0, ui::DANGER), WarText));
        });
}

/// No mar, perto de porto com a janela aberta: o placar da guerra.
fn update_war_panel(
    known: Res<KnownCompany>,
    docked: Res<MyDocked>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut panels: Query<&mut Node, With<WarPanel>>,
    mut texts: Query<&mut Text, With<WarText>>,
) {
    let me = my_ship.0.and_then(|id| {
        visuals
            .iter()
            .find(|visual| visual.target.ship_id == id)
            .map(|visual| Vec2::new(visual.target.x, visual.target.y))
    });
    let front = me.filter(|_| !docked.0).and_then(|at| {
        known
            .0
            .wars
            .iter()
            .filter(|front| front.open)
            .find(|front| at.distance(Vec2::new(front.x, front.y)) <= WAR_HUD_RANGE)
    });
    for mut node in &mut panels {
        let display = if front.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    let Some(front) = front else {
        return;
    };
    let flag = if known.0.tag.is_empty() {
        tr("você")
    } else {
        format!("[{}]", known.0.tag)
    };
    let value = trf(
        "GUERRA · {0} · {1} {2} × {3} rival",
        &[
            &tr(&front.port),
            &flag,
            &front.mine.to_string(),
            &front.rival.to_string(),
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
    use super::*;

    #[test]
    fn clock_reads_hours_or_minutes() {
        assert_eq!(clock_label(75.0), "1:15");
        assert_eq!(clock_label(3.0 * 3600.0 + 120.0), "3h02");
        assert_eq!(clock_label(-4.0), "0:00");
    }
}
