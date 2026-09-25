//! Central visual asset manifest (MF-056A, MF-057, MF-058). Systems consume
//! `GameAssets`; asset loading stays in this module instead of spreading
//! `asset_server.load` across visual systems.
//!
//! MF-058: o pack Scallywag e modular — casco, vela, verga e cesto de gavea
//! sao pecas separadas. Os recortes abaixo foram medidos no atlas (bounding
//! box de pixels opacos) e o navio e montado peca a peca em `ship.rs`.

use bevy::asset::{AssetServer, Handle, LoadState};
use bevy::prelude::*;
use bevy::sprite::TextureAtlasLayout;

/// Sheet do Scallywag com cascos, velas e vergas nossos
/// (`tools/art/marvyr_art.py`): cascos nos mesmos recortes do pack, velas e
/// vergas mais largas que o casco numa faixa nova a partir de y = 680.
const SHIP_SHEET: &str = "marvyr/ships/ships.png";
const WATER_AND_ISLANDS_SHEET: &str = "external/scallywag/water-islands/water-island-tiles.png";
/// Mesmo recorte do `fort-tiles.png` do Scallywag, com a pedra roxa
/// recolorida para pedra clara (`tools/art/marvyr_art.py`).
const FORT_SHEET: &str = "marvyr/world/fort-stone.png";
/// Construções do porto vistas de cima (`tools/art/marvyr_art.py`).
const BUILDINGS_SHEET: &str = "marvyr/world/port-buildings.png";

/// Ordem de desenho do mundo 2D. Sistemas visuais usam estes valores em vez
/// de espalhar profundidades numericas que podem inverter a cena por acaso.
pub mod layers {
    pub const OCEAN: f32 = -10.0;
    pub const WAKE: f32 = -9.5;
    pub const LAND: f32 = -9.0;
    pub const PROPS: f32 = -8.0;
    pub const RESOURCES: f32 = -7.0;
    pub const WRECKS: f32 = -6.0;
    pub const SHIPS: f32 = -5.0;
    pub const PROJECTILES: f32 = -4.0;
    pub const VFX: f32 = 0.0;
    pub const LABELS: f32 = 5.0;
    pub const HUD: f32 = 10.0;
    pub const OVERLAY: f32 = 20.0;
}

/// Tamanho do casco no atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HullSize {
    Small,
    Medium,
    Large,
}

/// Cor de casco (colunas do atlas): marrom, madeira clara, vermelho, azul
/// marinho, laranja com friso dourado.
pub const HULL_COLORS: usize = 5;
/// Cor de vela/verga/cesto (colunas do atlas): branco, creme, verde,
/// amarelo, azul, vermelho.
pub const SAIL_COLORS: usize = 6;

/// Indices no layout `ship_parts`. A ordem de insercao em
/// [`ship_parts_layout`] e a fonte da verdade; estes helpers a espelham.
pub mod parts {
    use super::{HullSize, HULL_COLORS, SAIL_COLORS};

    const HULLS: usize = 0; // 3 tamanhos x 5 cores x (intacto, avariado)
    const SAILS: usize = HULLS + 3 * HULL_COLORS * 2; // 3 x 6 x (recolhida, cheia)
    const YARDS: usize = SAILS + 3 * SAIL_COLORS * 2; // 3 x 6
    const NESTS: usize = YARDS + 3 * SAIL_COLORS; // 6
    pub const BOW_WAVE: usize = NESTS + SAIL_COLORS; // 3 frames
    pub const SMOKE: usize = BOW_WAVE + 3; // 4 frames
    pub const FIRE: usize = SMOKE + 4; // 4 frames
    pub const COUNT: usize = FIRE + 4;

    fn size_index(size: HullSize) -> usize {
        match size {
            HullSize::Small => 0,
            HullSize::Medium => 1,
            HullSize::Large => 2,
        }
    }

    pub fn hull(size: HullSize, color: usize, damaged: bool) -> usize {
        HULLS + (size_index(size) * HULL_COLORS + color % HULL_COLORS) * 2 + damaged as usize
    }

    pub fn sail(size: HullSize, color: usize, full: bool) -> usize {
        SAILS + (size_index(size) * SAIL_COLORS + color % SAIL_COLORS) * 2 + full as usize
    }

    pub fn yard(size: HullSize, color: usize) -> usize {
        YARDS + size_index(size) * SAIL_COLORS + color % SAIL_COLORS
    }

    pub fn nest(color: usize) -> usize {
        NESTS + color % SAIL_COLORS
    }
}

/// Indices no layout `deco` (water-islands sheet).
pub mod deco {
    pub const ROCK: usize = 0;
    pub const ROCK_B: usize = 1;
    pub const ROCK_MOSS: usize = 2;
    pub const ROCK_MOSS_B: usize = 3;
    pub const ROWBOAT: usize = 4;
    pub const PALM: usize = 5;
    pub const PALM_B: usize = 6;
    pub const BUSH: usize = 7;
    pub const BUSH_B: usize = 8;
    pub const LAMP: usize = 9;
    pub const PLANK: usize = 10;
    pub const PLANK_B: usize = 11;
    pub const PLANK_DIAG: usize = 12;
    pub const CHEST: usize = 13;
    pub const CHEST_GOLD: usize = 14;
}

/// Indices no layout `fort_parts`.
pub mod fort {
    pub const TOWER: usize = 0;
    pub const TOWER_PLAIN: usize = 1;
    pub const WALL_BLOCK: usize = 2;
    pub const CRATE: usize = 3;
    pub const DOCK: usize = 4;
    pub const BOARDWALK: usize = 5;
    pub const BARREL: usize = 6;
    pub const CANNON: usize = 7;
    /// 6 cores x 3 frames de bandeira tremulando.
    pub const FLAG: usize = 8;
}

/// Índices no layout `buildings` (ordem de [`buildings_layout`]).
pub mod building {
    pub const WAREHOUSE: usize = 0;
    pub const HOUSE_RED: usize = 1;
    pub const HOUSE_THATCH: usize = 2;
    pub const TAVERN: usize = 3;
    pub const STALL: usize = 4;
}

#[derive(Resource)]
pub struct GameAssets {
    pub ships: Handle<Image>,
    pub ship_parts: Handle<TextureAtlasLayout>,
    pub water_and_islands: Handle<Image>,
    pub deco: Handle<TextureAtlasLayout>,
    pub fort: Handle<Image>,
    pub fort_parts: Handle<TextureAtlasLayout>,
    pub buildings: Handle<Image>,
    pub building_parts: Handle<TextureAtlasLayout>,
}

pub struct AssetManifestPlugin;

pub fn image_failed(asset_server: &AssetServer, image: &Handle<Image>) -> bool {
    matches!(
        asset_server.get_load_state(image.id()),
        Some(LoadState::Failed(_))
    )
}

impl Plugin for AssetManifestPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_game_assets)
            .add_systems(Update, report_asset_load_result);
    }
}

fn rect(x: u32, y: u32, w: u32, h: u32) -> URect {
    URect::new(x, y, x + w, y + h)
}

pub fn ship_parts_layout() -> TextureAtlasLayout {
    let mut layout = TextureAtlasLayout::new_empty(UVec2::new(720, 800));
    // Cascos: (x0, passo, largura, altura). Avariado fica logo abaixo.
    for (x0, w, h) in [(1, 30, 64), (162, 44, 80), (401, 46, 128)] {
        let step = if w == 30 { 32 } else { 48 };
        for color in 0..HULL_COLORS as u32 {
            for damaged in 0..2 {
                layout.add_texture(rect(x0 + color * step, damaged * h, w, h));
            }
        }
    }
    // Velas (faixa nova do gerador): uma linha por tamanho, passo de 120
    // px por cor, recolhida em +0 e cheia em +60.
    for (size, (w, h)) in [(36, 11), (52, 14), (58, 17)].into_iter().enumerate() {
        let y = 680 + size as u32 * 20;
        for color in 0..SAIL_COLORS as u32 {
            layout.add_texture(rect(color * 120, y, w, h));
            layout.add_texture(rect(color * 120 + 60, y, w, h));
        }
    }
    // Vergas (o "T" de mastro visto de cima), passo de 60 px por cor.
    for (size, (w, h)) in [(36, 12), (52, 12), (58, 15)].into_iter().enumerate() {
        let y = 745 + size as u32 * 17;
        for color in 0..SAIL_COLORS as u32 {
            layout.add_texture(rect(color * 60, y, w, h));
        }
    }
    for color in 0..SAIL_COLORS as u32 {
        layout.add_texture(rect(4 + color * 48, 612, 24, 24));
    }
    for frame in 0..3 {
        layout.add_texture(rect(352 + frame * 32, 558, 24, 52));
    }
    for frame in 0..4 {
        layout.add_texture(rect(448 + frame * 16, 609, 16, 15));
    }
    for frame in 0..4 {
        layout.add_texture(rect(448 + frame * 16, 624, 16, 17));
    }
    debug_assert_eq!(layout.textures.len(), parts::COUNT);
    layout
}

pub fn deco_layout() -> TextureAtlasLayout {
    let mut layout = TextureAtlasLayout::new_empty(UVec2::new(384, 144));
    for r in [
        rect(1, 98, 15, 13),    // ROCK
        rect(17, 98, 15, 14),   // ROCK_B
        rect(48, 129, 16, 15),  // ROCK_MOSS
        rect(33, 114, 15, 14),  // ROCK_MOSS_B
        rect(64, 97, 32, 14),   // ROWBOAT
        rect(144, 97, 16, 16),  // PALM
        rect(128, 99, 16, 13),  // PALM_B
        rect(112, 112, 16, 16), // BUSH
        rect(97, 114, 14, 14),  // BUSH_B
        rect(163, 98, 9, 13),   // LAMP
        rect(96, 132, 16, 8),   // PLANK
        rect(113, 133, 13, 7),  // PLANK_B
        rect(67, 132, 10, 12),  // PLANK_DIAG
        rect(210, 35, 13, 11),  // CHEST
        rect(226, 49, 13, 15),  // CHEST_GOLD
    ] {
        layout.add_texture(r);
    }
    layout
}

pub fn fort_parts_layout() -> TextureAtlasLayout {
    let mut layout = TextureAtlasLayout::new_empty(UVec2::new(432, 256));
    for r in [
        rect(192, 96, 32, 32),  // TOWER
        rect(192, 128, 32, 32), // TOWER_PLAIN
        rect(0, 96, 32, 32),    // WALL_BLOCK
        rect(192, 160, 32, 32), // CRATE
        rect(144, 200, 64, 40), // DOCK
        rect(208, 192, 64, 48), // BOARDWALK
        rect(290, 82, 12, 13),  // BARREL
        rect(305, 17, 31, 15),  // CANNON
    ] {
        layout.add_texture(r);
    }
    for color in 0..SAIL_COLORS as u32 {
        for frame in 0..3 {
            layout.add_texture(rect(392 + frame * 16, 99 + color * 16, 8, 9));
        }
    }
    layout
}

/// Recortes impressos pelo gerador (`tools/art/marvyr_art.py`).
pub fn buildings_layout() -> TextureAtlasLayout {
    let mut layout = TextureAtlasLayout::new_empty(UVec2::new(162, 32));
    for r in [
        rect(0, 0, 44, 30),   // WAREHOUSE
        rect(46, 0, 26, 22),  // HOUSE_RED
        rect(74, 0, 22, 26),  // HOUSE_THATCH
        rect(98, 0, 38, 32),  // TAVERN
        rect(138, 0, 22, 19), // STALL
    ] {
        layout.add_texture(r);
    }
    layout
}

pub(crate) fn load_game_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    commands.insert_resource(GameAssets {
        ships: asset_server.load(SHIP_SHEET),
        ship_parts: layouts.add(ship_parts_layout()),
        water_and_islands: asset_server.load(WATER_AND_ISLANDS_SHEET),
        deco: layouts.add(deco_layout()),
        fort: asset_server.load(FORT_SHEET),
        fort_parts: layouts.add(fort_parts_layout()),
        buildings: asset_server.load(BUILDINGS_SHEET),
        building_parts: layouts.add(buildings_layout()),
    });
}

fn report_asset_load_result(
    asset_server: Res<AssetServer>,
    assets: Res<GameAssets>,
    mut reported: Local<bool>,
) {
    if *reported {
        return;
    }

    let sheets = [
        ("ships", &assets.ships),
        ("water-and-islands", &assets.water_and_islands),
        ("fort", &assets.fort),
        ("buildings", &assets.buildings),
    ];
    for (pack, sheet) in sheets {
        if let Some(LoadState::Failed(error)) = asset_server.get_load_state(sheet.id()) {
            warn!(pack, error = %error, "visual asset sheet failed to load; affected entities use geometric fallback");
            *reported = true;
            return;
        }
    }

    if sheets.iter().all(|(_, sheet)| {
        matches!(
            asset_server.get_load_state(sheet.id()),
            Some(LoadState::Loaded)
        )
    }) {
        info!("visual asset sheets loaded");
        *reported = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_indices_match_layout_insertion_order() {
        let layout = ship_parts_layout();
        assert_eq!(layout.textures.len(), parts::COUNT);
        // Casco medio intacto da 2a cor comeca em x=210 (passo de 48 px).
        let medium = layout.textures[parts::hull(HullSize::Medium, 1, false)];
        assert_eq!((medium.min.x, medium.min.y, medium.height()), (210, 0, 80));
        let damaged = layout.textures[parts::hull(HullSize::Large, 0, true)];
        assert_eq!((damaged.min.y, damaged.height()), (128, 128));
        let full_red = layout.textures[parts::sail(HullSize::Large, 5, true)];
        assert_eq!((full_red.min.x, full_red.min.y), (660, 720));
        let nest = layout.textures[parts::nest(0)];
        assert_eq!((nest.min.x, nest.min.y), (4, 612));
    }

    #[test]
    fn every_rect_fits_its_sheet() {
        for (layout, (w, h)) in [
            (ship_parts_layout(), (720, 800)),
            (deco_layout(), (384, 144)),
            (fort_parts_layout(), (432, 256)),
            (buildings_layout(), (162, 32)),
        ] {
            for r in &layout.textures {
                assert!(r.max.x <= w && r.max.y <= h, "{r:?}");
            }
        }
    }
}
