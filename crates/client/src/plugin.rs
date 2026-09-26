use bevy::prelude::*;

use crate::assets::AssetManifestPlugin;
use crate::audio::SoundPlugin;
use crate::camera::{follow_camera, setup_camera, zoom_from_wheel, CameraZoom};
use crate::crafting::{send_craft_input, CraftPlugin};
use crate::hud::{toggle_sea_hud, HudPlugin};
use crate::juice::JuicePlugin;
use crate::market::{send_market_input, MarketPlugin};
use crate::net::ClientNetPlugin;
use crate::nodes::NodePlugin;
use crate::port_screen::PortPlugin;
use crate::ship::{
    animate_loot_beams, animate_ship_parts, animate_sinking, draw_gunnery, emit_foam,
    expire_stale_visuals, lerp_projectile_visuals, lerp_ship_visuals, update_wanted_markers,
    upsert_projectile_visuals, upsert_ship_visuals, upsert_wreck_visuals,
};
use crate::ui::UiThemePlugin;
use crate::vfx::VfxPlugin;
use crate::wanted_hud::WantedHudPlugin;
use crate::weather::WeatherPlugin;
use crate::world::WorldVisualPlugin;
use crate::zone::ZonePlugin;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.08, 0.30, 0.56)))
            .init_resource::<CameraZoom>()
            // ADR-0008: simulacao a 30 Hz; render desacoplado.
            .insert_resource(Time::<Fixed>::from_hz(30.0))
            .add_plugins(AssetManifestPlugin)
            .add_plugins(ClientNetPlugin)
            .add_plugins(ZonePlugin)
            .add_plugins(WorldVisualPlugin)
            .add_plugins(NodePlugin)
            .add_plugins(CraftPlugin)
            .add_plugins(MarketPlugin)
            .add_plugins(PortPlugin)
            .add_plugins(crate::affixes::AffixPlugin)
            .add_plugins(crate::gems::GemsPlugin)
            .add_plugins(crate::flasks::FlasksPlugin)
            .add_plugins(crate::inventory::InventoryPlugin)
            .add_plugins(HudPlugin)
            .add_plugins(VfxPlugin)
            .add_plugins(WeatherPlugin)
            .add_plugins(UiThemePlugin)
            .add_plugins(crate::i18n::I18nPlugin)
            .add_plugins(crate::input::GamepadPlugin)
            .add_plugins(crate::help::HelpPlugin)
            .add_plugins(crate::onboarding::OnboardingPlugin)
            .add_plugins(crate::portals::PortalClientPlugin)
            .add_plugins(crate::chart::ChartPlugin)
            .add_plugins(crate::renown::RenownPlugin)
            .add_plugins(crate::talents::TalentsPlugin)
            .add_plugins(JuicePlugin)
            .add_plugins(SoundPlugin)
            .add_plugins(WantedHudPlugin)
            .add_plugins(crate::seafaring::SeafaringPlugin)
            .add_plugins(crate::blood_tide::BloodTidePlugin)
            .add_plugins(crate::session::SessionPlugin)
            .add_systems(Update, update_wanted_markers.after(lerp_ship_visuals))
            .add_systems(
                Update,
                crate::ship::fix_ship_shadows.after(lerp_ship_visuals),
            )
            .add_systems(Startup, setup_camera)
            .add_systems(
                Update,
                (
                    upsert_ship_visuals,
                    lerp_ship_visuals,
                    upsert_projectile_visuals,
                    lerp_projectile_visuals,
                    upsert_wreck_visuals,
                    animate_loot_beams,
                    expire_stale_visuals,
                    animate_ship_parts,
                    animate_sinking,
                    emit_foam,
                    (
                        draw_gunnery,
                        crate::ship::puff_on_black_flag,
                        crate::ship::animate_auras,
                    ),
                    toggle_sea_hud,
                    zoom_from_wheel,
                    follow_camera.after(lerp_ship_visuals),
                    send_craft_input,
                    send_market_input,
                ),
            );
    }
}
