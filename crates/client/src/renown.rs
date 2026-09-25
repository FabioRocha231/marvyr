//! Renome no client (MV-067): barra no painel do navio, "+N Renome" subindo
//! do casco a cada feito e a faixa de nível novo. O servidor conta; aqui só
//! se mostra.

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::RenownUpdate;

use crate::hud::ZoneBannerAnchor;
use crate::i18n::{tr, trf};
use crate::net::MyShip;
use crate::ship::ShipVisual;
use crate::ui;

#[derive(Resource, Debug, Default)]
pub struct MyRenown(pub Option<RenownUpdate>);

pub struct RenownPlugin;

impl Plugin for RenownPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MyRenown>()
            .add_systems(Update, receive_renown);
    }
}

/// "Nv 4 · 35/150" (ou "Nv 30 · máx").
pub fn level_label(update: &RenownUpdate) -> String {
    if update.span == 0 {
        trf("Nv {0} · máx", &[&update.level.to_string()])
    } else {
        trf(
            "Nv {0} · {1}/{2}",
            &[
                &update.level.to_string(),
                &update.into.to_string(),
                &update.span.to_string(),
            ],
        )
    }
}

/// Quanto da barra do nível está cheio (nível máximo = cheia).
pub fn level_fraction(update: &RenownUpdate) -> f32 {
    if update.span == 0 {
        1.0
    } else {
        (update.into as f32 / update.span as f32).clamp(0.0, 1.0)
    }
}

fn receive_renown(
    mut commands: Commands,
    mut events: EventReader<ClientReceiveMessage<RenownUpdate>>,
    mut mine: ResMut<MyRenown>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    banner: Query<Entity, With<ZoneBannerAnchor>>,
) {
    for event in events.read() {
        let update = event.message().clone();
        let previous = mine.0.as_ref().map(|old| old.level);
        if update.gained > 0 {
            let hull = my_ship.0.and_then(|id| {
                visuals
                    .iter()
                    .find(|visual| visual.target.ship_id == id)
                    .map(|visual| Vec2::new(visual.target.x, visual.target.y))
            });
            if let Some(at) = hull {
                crate::juice::spawn_float_text(
                    &mut commands,
                    at + Vec2::new(0.0, 18.0),
                    // O motivo junto: "subi de nível sem saber como" não
                    // pode acontecer.
                    trf(
                        "+{0} Renome · {1}",
                        &[&update.gained.to_string(), &tr(&update.reason)],
                    ),
                    ui::BRASS,
                );
            }
        }
        let leveled = previous.is_some_and(|level| update.level > level);
        if let (true, Ok(anchor)) = (leveled, banner.get_single()) {
            commands.entity(anchor).despawn_descendants();
            crate::hud::spawn_level_banner(&mut commands, anchor, update.level);
            info!(level = update.level, reason = %tr(&update.reason), "Renome subiu de nível");
        }
        mine.0 = Some(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(into: u64, span: u64) -> RenownUpdate {
        RenownUpdate {
            total: 0,
            level: 4,
            into,
            span,
            gained: 0,
            reason: String::new(),
        }
    }

    #[test]
    fn label_and_bar_follow_the_level_progress() {
        assert!(level_label(&update(35, 150)).contains("35/150"));
        assert!((level_fraction(&update(75, 150)) - 0.5).abs() < 1e-6);
        assert_eq!(level_fraction(&update(0, 0)), 1.0);
        assert!(level_label(&update(0, 0)).contains("máx"));
    }
}
