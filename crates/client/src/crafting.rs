//! Crafting no client (PRD MF-021/022). O catálogo de receitas chega no
//! handshake e alimenta os atalhos `CraftItem` (teclas 1-9 ou autocraft dev);
//! a lista visual pertence à tela de porto (MF-042). Estação, ingredientes e
//! porão são julgados pelo servidor.

use bevy::ecs::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_protocol::{CraftItem, CraftResult, RecipeEntry, RecipesSnapshot};

use crate::net::ReliableChannel;

/// Receitas conhecidas (do RecipesSnapshot do handshake).
#[derive(Resource, Debug, Default)]
pub struct KnownRecipes(pub Vec<RecipeEntry>);

pub struct CraftPlugin;

impl Plugin for CraftPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownRecipes>()
            .add_systems(Update, (handle_recipes_snapshot, handle_craft_result));
    }
}

/// Catálogo do servidor chega: guarda para os atalhos de craft. A lista
/// visual pertence à tela de porto (MF-042), não ao HUD do mar.
fn handle_recipes_snapshot(
    mut snapshot_events: EventReader<ClientReceiveMessage<RecipesSnapshot>>,
    mut known: ResMut<KnownRecipes>,
) {
    for event in snapshot_events.read() {
        known.0 = event.message().recipes.clone();
        info!(recipes = known.0.len(), "catálogo de receitas recebido");
    }
}

fn handle_craft_result(
    mut events: EventReader<ClientReceiveMessage<CraftResult>>,
    mut notices: EventWriter<crate::net::PlayerNotice>,
) {
    for event in events.read() {
        let result = event.message();
        if result.success {
            info!(
                recipe_id = result.recipe_id,
                "fabricação concluída pelo servidor"
            );
        } else {
            warn!(
                recipe_id = result.recipe_id,
                reason = %result.reason,
                "fabricação recusada pelo servidor"
            );
        }
        notices.send_batch(crate::net::PlayerNotice::from_refusal(
            result.success,
            &result.reason,
        ));
    }
}

/// Teclas 1-9 fabricam a receita correspondente. Dev tooling (§39):
/// MARVYR_AUTOCRAFT=1 tenta a lista em ciclo — smoke sem interação.
pub fn send_craft_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    known: Res<KnownRecipes>,
    mut auto_timer: Local<f32>,
    mut auto_index: Local<u32>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let number_keys = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    let mut selected = number_keys
        .iter()
        .position(|key| keys.just_pressed(*key))
        .map(|index| index as u32);

    if autocraft_enabled() {
        // Ciclo de 1s: com 5 receitas, a mesma cai a cada 5s — em tempo de
        // estar com recurso e estação ao mesmo tempo (janela da baía).
        *auto_timer += time.delta_secs();
        if *auto_timer >= 1.0 && !known.0.is_empty() {
            *auto_timer = 0.0;
            selected = Some(*auto_index % known.0.len() as u32);
            *auto_index = (*auto_index + 1) % known.0.len() as u32;
        }
    }

    if let Some(recipe_id) = selected {
        if known.0.iter().any(|entry| entry.recipe_id == recipe_id) {
            info!(recipe_id, "fabricando");
            let _ = connection_manager.send_message::<ReliableChannel, _>(&CraftItem {
                recipe_id,
                rarity: autocraft_rarity(),
            });
        }
    }
}

fn autocraft_enabled() -> bool {
    std::env::var_os("MARVYR_AUTOCRAFT").is_some()
}

/// Dev: `MARVYR_AUTOCRAFT=rare|magic` fabrica na raridade (smoke dos afixos).
fn autocraft_rarity() -> marvyr_domain_items::Rarity {
    match std::env::var("MARVYR_AUTOCRAFT").as_deref() {
        Ok("rare") => marvyr_domain_items::Rarity::Rare,
        Ok("magic") => marvyr_domain_items::Rarity::Magic,
        _ => marvyr_domain_items::Rarity::Normal,
    }
}
