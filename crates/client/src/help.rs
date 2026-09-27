//! Livreto do marujo (MV-062): F1 / Esc / Start abre, em qualquer lugar do
//! mar. Um livreto encadernado — capa de tipo de madeira, páginas por
//! assunto, teclas do dispositivo ativo (teclado ou controle), opções de
//! idioma, rever o guia e sair do jogo. B/Esc volta exatamente pelo caminho
//! que o jogador fez; na primeira página, fecha.

use bevy::prelude::*;

use crate::i18n::{self, ChangeLang, Lang};
use crate::input::{glyph, InputDevice, ModalOpen};
use crate::net::MyDocked;
use crate::session::ConnectionStatus;
use crate::ui;

/// No browser o `AppExit` só congela o canvas (a aba é quem fecha o jogo).
const CAN_QUIT: bool = !cfg!(target_arch = "wasm32");

/// Páginas, na ordem das abas.
const PAGES: [&str; 8] = [
    "Navegar",
    "Combate",
    "Porto e comércio",
    "O mar",
    // Renome, Diário, mentoria, frete e guilda ("O mar" passava do papel).
    "Progresso",
    // Party, sinais, companhia e o mar compartilhado (a página Combate
    // passava do papel).
    "Aliados",
    "Controle",
    "Opções",
];
const OPTIONS_PAGE: usize = 7;
const OPTION_ROWS: usize = 4;

#[derive(Resource, Debug, Default)]
pub struct HelpBook {
    pub open: bool,
    pub page: usize,
    /// Páginas visitadas, para B/Esc voltar pelo mesmo caminho.
    trail: Vec<usize>,
    /// Linha escolhida na página de opções.
    row: usize,
    /// Dev: a captura já abriu o livreto uma vez.
    shot_done: bool,
}

impl HelpBook {
    pub fn open_at(&mut self, page: usize) {
        self.open = true;
        self.page = page.min(PAGES.len() - 1);
        self.trail.clear();
        self.row = 0;
    }

    fn turn(&mut self, forward: bool) {
        let next = if forward {
            (self.page + 1) % PAGES.len()
        } else {
            (self.page + PAGES.len() - 1) % PAGES.len()
        };
        self.trail.push(self.page);
        self.page = next;
        self.row = 0;
    }

    /// Volta uma página no caminho; `false` quando não há para onde voltar.
    fn back(&mut self) -> bool {
        match self.trail.pop() {
            Some(page) => {
                self.page = page;
                self.row = 0;
                true
            }
            None => false,
        }
    }
}

/// Pedido do livreto para o guia recomeçar (tratado em `onboarding`).
#[derive(Event, Debug, Clone, Copy)]
pub struct RestartGuide;

#[derive(Component)]
struct HelpRoot;
#[derive(Component)]
struct HelpBody;
#[derive(Component)]
struct HelpTab(usize);
#[derive(Component)]
struct HelpFooter;

pub struct HelpPlugin;

impl Plugin for HelpPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HelpBook>()
            .add_event::<RestartGuide>()
            .add_systems(Startup, spawn_book)
            .add_systems(
                Update,
                (
                    handle_book_keys,
                    sync_modal,
                    (show_book, draw_tabs, draw_page, draw_footer),
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
pub(crate) fn spawn_book_for_tests(commands: Commands) {
    spawn_book(commands);
}

fn spawn_book(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(ui::INK.with_alpha(0.55)),
            GlobalZIndex(40),
            HelpRoot,
        ))
        .with_children(|scrim| {
            scrim
                .spawn((ui::panel(Node {
                    width: Val::Px(760.0),
                    max_width: Val::Percent(94.0),
                    max_height: Val::Percent(90.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(20.0)),
                    row_gap: Val::Px(12.0),
                    overflow: Overflow::clip(),
                    ..default()
                }),))
                .with_children(|book| {
                    ui::masthead(book, "LIVRETO DO MARUJO", 34.0);
                    ui::double_rule(book);
                    book.spawn(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(18.0),
                        row_gap: Val::Px(4.0),
                        ..default()
                    })
                    .with_children(|tabs| {
                        for (index, title) in PAGES.iter().enumerate() {
                            tabs.spawn((
                                ui::face(*title, ui::FONT_BOLD, 16.0, ui::INK_SOFT),
                                i18n::Translated(title),
                                HelpTab(index),
                            ));
                        }
                    });
                    book.spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            min_height: Val::Px(300.0),
                            // Página que passar do papel é cortada, nunca
                            // escrita por cima do rodapé.
                            flex_grow: 1.0,
                            flex_shrink: 1.0,
                            overflow: Overflow::clip_y(),
                            ..default()
                        },
                        HelpBody,
                    ));
                    ui::rule(book);
                    book.spawn((ui::text("", 13.0, ui::INK_SOFT), HelpFooter));
                });
        });
}

#[allow(clippy::too_many_arguments)]
fn handle_book_keys(
    raw_keys: Res<ButtonInput<KeyCode>>,
    modal: Res<ModalOpen>,
    captured: Res<crate::input::ModalKeys>,
    status: Res<ConnectionStatus>,
    docked: Res<MyDocked>,
    lang: Res<Lang>,
    mut book: ResMut<HelpBook>,
    mut change_lang: EventWriter<ChangeLang>,
    mut restart: EventWriter<RestartGuide>,
    mut exit: EventWriter<AppExit>,
    mut logout: EventWriter<crate::session::Logout>,
) {
    let keys = crate::input::modal_keys(&modal, &captured, &raw_keys);
    if *status != ConnectionStatus::InGame {
        // Fora do mar (login, erro de conexão) Esc continua saindo do jogo;
        // L aqui é letra do formulário, não troca de idioma.
        if keys.just_pressed(KeyCode::Escape) && !book.open && CAN_QUIT {
            exit.send(AppExit::Success);
        }
        book.open = false;
        return;
    }
    if modal.welcome {
        return; // o cartaz de boas-vindas é dono das teclas
    }
    if keys.just_pressed(KeyCode::KeyL) {
        change_lang.send(ChangeLang(lang.toggled()));
    }
    if !book.open {
        // Dev (captura de tela): MARVYR_SHOT_HELP=<página> abre o livreto
        // uma vez ao entrar no mar.
        let shot_page = std::env::var("MARVYR_SHOT_HELP")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|_| !book.shot_done);
        if let Some(page) = shot_page {
            book.shot_done = true;
            book.open_at(page);
            return;
        }
        let wants =
            keys.just_pressed(KeyCode::F1) || (keys.just_pressed(KeyCode::Escape) && !docked.0);
        if wants {
            book.open_at(0);
        }
        return;
    }
    if keys.just_pressed(KeyCode::F1) {
        book.open = false;
        return;
    }
    if keys.just_pressed(KeyCode::Escape) && !book.back() {
        book.open = false;
        return;
    }
    if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::Tab) {
        let backward = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        book.turn(!backward);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        book.turn(false);
    }
    if book.page == OPTIONS_PAGE {
        if keys.just_pressed(KeyCode::ArrowDown) {
            book.row = (book.row + 1) % OPTION_ROWS;
        }
        if keys.just_pressed(KeyCode::ArrowUp) {
            book.row = (book.row + OPTION_ROWS - 1) % OPTION_ROWS;
        }
        if keys.just_pressed(KeyCode::Enter) {
            match book.row {
                0 => {
                    change_lang.send(ChangeLang(lang.toggled()));
                }
                1 => {
                    restart.send(RestartGuide);
                    book.open = false;
                }
                2 => {
                    logout.send(crate::session::Logout);
                    book.open = false;
                }
                _ if CAN_QUIT => {
                    exit.send(AppExit::Success);
                }
                _ => {}
            }
        }
    }
}

fn sync_modal(book: Res<HelpBook>, mut modal: ResMut<ModalOpen>) {
    if modal.book != book.open {
        modal.book = book.open;
    }
}

fn show_book(book: Res<HelpBook>, mut root: Query<&mut Node, With<HelpRoot>>) {
    if !book.is_changed() {
        return;
    }
    for mut node in &mut root {
        node.display = if book.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn draw_tabs(book: Res<HelpBook>, mut tabs: Query<(&HelpTab, &mut TextColor)>) {
    if !book.is_changed() {
        return;
    }
    for (tab, mut color) in &mut tabs {
        color.0 = if tab.0 == book.page {
            ui::VERMILION_INK
        } else {
            ui::INK_SOFT
        };
    }
}

fn draw_footer(
    book: Res<HelpBook>,
    device: Res<InputDevice>,
    lang: Res<Lang>,
    mut footer: Query<&mut Text, With<HelpFooter>>,
) {
    if !(book.is_changed() || device.is_changed() || lang.is_changed()) {
        return;
    }
    let line = match *device {
        InputDevice::Keyboard => {
            i18n::tr("Setas ou Tab: páginas   ·   Esc: voltar   ·   F1: fechar   ·   L: idioma")
        }
        InputDevice::Gamepad => i18n::tr(
            "Direcional ou LB/RB: páginas   ·   B: voltar   ·   Start: fechar   ·   Select: idioma",
        ),
    };
    for mut text in &mut footer {
        text.0 = line.clone();
    }
}

/// Uma linha do livreto: tecla (no dispositivo ativo) e o que ela faz.
enum Line {
    Key(KeyCode, &'static str),
    /// Tecla com rótulo próprio (roda do mouse, analógico direito…).
    Custom(&'static str, &'static str, &'static str),
    Prose(&'static str),
    Heading(&'static str),
}

fn page_lines(page: usize) -> Vec<Line> {
    use Line::*;
    match page {
        0 => vec![
            Key(KeyCode::KeyW, "Içar vela (três níveis: meia, cruzeiro, cheia)"),
            Key(KeyCode::KeyS, "Recolher vela"),
            Custom("A / D", "Analógico esquerdo", "Leme: virar a bombordo e a boreste"),
            Custom("Roda do mouse", "Analógico direito", "Aproximar e afastar a câmera"),
            Key(KeyCode::KeyE, "Atracar quando o bilhete de porto aparecer"),
            // Só teclado: o controle não tem botão sobrando para a carta.
            Custom("M", "—", "Carta náutica: o que você já navegou"),
            Custom("O", "—", "Ligar e desligar a música"),
            Custom("I", "—", "Rosa dos Ventos: talentos ganhos com Renome"),
            Custom("F2", "—", "Diário de Bordo: metas do dia e da semana"),
            Prose("O painel no canto direito mostra suas velas e a munição. Tempestade rasga o pano e o navio fica lento; no mar ele se remenda devagar, atracado na hora."),
        ],
        1 => vec![
            Custom("Clique esquerdo", "RT", "Salva do bordo que encara o cursor; segure para repetir"),
            Custom("Clique direito", "LT", "Abalroar: arrancada que também esquiva; o choque machuca"),
            Custom("Z", "L3", "Salva em leque na direção do cursor"),
            Custom("X", "R3", "Barril incendiário na esteira: explode em quem passar"),
            Key(KeyCode::KeyQ, "Travar o alvo do tiro automático (ou soltar)"),
            Key(KeyCode::KeyR, "Içar ou arriar a Bandeira Negra"),
            Key(KeyCode::KeyC, "Trocar a munição"),
            Key(KeyCode::KeyK, "Reparar no mar: gasta madeira, só parado e fora de combate"),
            Key(KeyCode::KeyH, "Abordar um navio avariado, lado a lado e devagar"),
            Custom("1 a 3", "—", "No duelo de abordagem: Assalto vence Mosquete, Mosquete vence Muralha, Muralha vence Assalto. O pirata grita o que vai fazer, mas às vezes blefa"),
            Key(KeyCode::KeyF, "Saquear um destroço (passar por cima já recolhe)"),
            // v25: só teclado por ora — o controle não tem botão sobrando.
            Custom("1 a 4", "—", "Frascos do porão: Estopa, Vento, Fúria e Breu. Acertos recarregam; o porto enche"),
            Prose("Sem mirar, os canhões atiram sozinhos e mais fracos no inimigo mais perto. Inocente só com alvo travado ou de Bandeira Negra (aí todos miram em você). Tiro na popa avaria o leme; pirata afundado solta recurso e, às vezes, Estilhaço de Gema e orbe."),
        ],
        2 => vec![
            Key(KeyCode::KeyG, "Coletar no ponto de recurso marcado no mar"),
            Key(KeyCode::KeyE, "Atracar e desatracar"),
            Key(KeyCode::Tab, "Trocar de aba no porto (porão, mercado, fabricação…)"),
            Key(KeyCode::Enter, "Confirmar a linha escolhida"),
            Key(KeyCode::KeyP, "Contratar marujos (atracado, paga em Madeira)"),
            Custom("Shift+P", "—", "Contratar oficial (atracado): Artilheiro recarrega mais rápido (20 Minério), Contramestre firma o leme (20 Madeira), Cirurgião salva metade das baixas (10 Coral). Afundou, foram junto; abordagem vencida captura um do rendido"),
            Prose("Não há moeda: tudo se troca. Tudo que vale algo é fabricado por jogadores. Colete, fabrique, carregue o porão e troque onde o seu recurso vale mais. O caminho entre os portos é o risco — e o lucro."),
        ],
        3 => vec![
            Heading("Zonas"),
            Prose("Águas protegidas: ninguém ataca você. Fronteira: combate liberado. Sem lei: combate e saque total — afundou, a carga vira destroço de quem pegar."),
            Heading("Portais"),
            Prose("Cerração: banco de névoa com tempo e vagas contados; leva a uma arena isolada, diferente a cada abertura, com baús de Cristal da Cerração que afundam em 5 minutos. Devolve você quando se dissipa. Sorvedouro: redemoinho que liga pontos distantes por dentro de águas sem lei."),
            Heading("Eventos de mar"),
            Prose("Tormenta desgasta o casco de quem está dentro. Frota do tesouro navega com escolta. O kraken morde quem chega perto. Maré disputada faz brotar recurso raro em mar aberto."),
            Heading("Tesouro"),
            Prose("Às vezes a coleta rende um mapa. Leve-o até o X marcado no mar, pare o navio e cave."),
            Prose("Correntes: toda semana cada zona ganha uma faixa de água rápida, com espuma correndo no sentido dela e pontilhada na carta. A favor, o navio voa; contra, rema."),
            Prose("Moral: longe do porto a tripulação desanima, mais rápido à noite, e o navio anda menos. Ela come Peixe do porão sozinha, se recupera na luz de um farol e enche ao atracar."),
            Prose("Boca do Abismo: redemoinho violeta no mar sem lei. Entre no anel e desça: cada camada solta saqueadores de elite mais fortes, com 90 s para vencer. Cada camada vencida deixa um destroço só seu, mais rico quanto mais fundo. Fugiu, afundou ou estourou o tempo, o Abismo te cospe."),
        ],
        4 => vec![
            Heading("Renome e nível"),
            Prose("Seu nível é o Renome do capitão. Ele sobe coletando, fabricando, construindo navio, entregando contrato, saqueando destroço e afundando navio. Cada nível dá um ponto na Rosa dos Ventos (tecla I)."),
            Prose("F2 abre o Diário de Bordo: metas do dia e da semana, o Livro de Bordo (coleção que rende títulos), a maestria de cada casco e a temporada. Cada temporada dura seis semanas, tem um tema e conta o Renome que você ganha; passe da coroa e o título Coroa da Maré é seu para sempre."),
            Prose("Mentoria: um capitão veterano (nível 10+) que navega perto de um novato (até nível 4) em águas protegidas vira mentor dele. O novato coleta 50% a mais e o mentor ganha Renome a cada minuto juntos."),
            Prose("Frete: na aba Frete do porto, anuncie carga para outro porto com um prêmio, ou leve a de outro capitão deixando uma caução. Entregou no prazo, o prêmio e a caução são seus; não entregou, ficam com o dono."),
            Prose("Guilda: item que um porto não recebe há horas fica EM FALTA; quem entrega ganha Renome além da troca."),
        ],
        5 => vec![
            Custom("V", "—", "Party: convida o capitão sob o cursor (ou o mais perto); com convite, aceita. Shift+V recusa ou sai. Na party ninguém acerta ninguém, o saque do vencedor vale para todos e afundar pirata dá Renome a quem estava perto"),
            Custom("F5 F6 F7", "—", "Sinal para a party e a companhia: Socorro, Ataquem aqui, Reagrupar em mim"),
            Prose("Companhia: funde na aba Companhia do porto (40 Madeira + 40 Minério). Membros são aliados, a tag vai na placa e a influência soma. Cada porto fora da coroa abre guerra 20 min a cada 3 h: segure o porto com a companhia e afunde rivais ali para tomá-lo; quem manda cobra o tributo do dia"),
            Custom("Espaço", "—", "Pescar: parado, lance a linha; quando morder, puxe"),
            Custom("F3", "—", "Mensagem na garrafa: monte uma frase e jogue ao mar; a corrente leva. Perto de uma garrafa boiando, B pesca e lê. Quem jogou ganha Renome quando alguém lê."),
            Custom("B", "A", "Farol: parado perto da costa, B ergue um farol com madeira e minério do porão; colado num farol aceso, B reforça com madeira. A luz aparece na carta de todos e quem ergueu ganha Renome a cada capitão que passa."),
        ],
        6 => vec![
            Custom("W / S", "Direcional cima e baixo", "Velas"),
            Custom("A / D", "Analógico esquerdo", "Leme"),
            Custom("Clique esq. / dir.", "RT / LT", "Salva mirada e abalroar"),
            Custom("Z / X", "L3 / R3", "Leque e barril"),
            Custom("Q / R", "LB / RB", "Travar alvo e Bandeira Negra"),
            Custom("E / G / F / H / J", "A", "Ação do bilhete: atracar, coletar, saquear, abordar, cavar"),
            Custom("K", "X", "Reparar"),
            Custom("C", "Y", "Munição"),
            Custom("F1 / Esc", "Start", "Este livreto"),
            Custom("L", "Select", "Idioma"),
            Prose("No porto: setas ou direcional escolhem, Enter ou A confirma, Tab ou LB/RB troca de aba, Esc ou B volta."),
        ],
        _ => Vec::new(),
    }
}

fn draw_page(
    mut commands: Commands,
    book: Res<HelpBook>,
    device: Res<InputDevice>,
    lang: Res<Lang>,
    body: Query<Entity, With<HelpBody>>,
) {
    if !(book.is_changed() || device.is_changed() || lang.is_changed()) || !book.open {
        return;
    }
    let Ok(body) = body.get_single() else {
        return;
    };
    commands.entity(body).despawn_descendants();
    commands.entity(body).with_children(|page| {
        if book.page == OPTIONS_PAGE {
            let rows = [
                i18n::trf("Idioma: {0}", &[lang.label()]),
                i18n::tr("Rever o guia da primeira viagem"),
                i18n::tr("Sair da conta"),
                i18n::tr("Sair do jogo"),
            ];
            for (index, label) in rows.into_iter().enumerate() {
                let chosen = index == book.row;
                page.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        border: UiRect::left(Val::Px(if chosen { 1.0 } else { 0.0 })),
                        ..default()
                    },
                    BackgroundColor(if chosen {
                        ui::BUTTON_SELECTED
                    } else {
                        Color::NONE
                    }),
                    BorderColor(ui::INK),
                ))
                .with_children(|row| {
                    row.spawn(ui::face(label, ui::FONT_BOLD, 18.0, ui::INK));
                });
            }
            let hint = match *device {
                InputDevice::Keyboard => "Setas escolhem · Enter confirma",
                InputDevice::Gamepad => "Direcional escolhe · A confirma",
            };
            page.spawn(ui::text(hint, 14.0, ui::INK_SOFT));
            return;
        }
        for line in page_lines(book.page) {
            match line {
                Line::Key(key, what) => {
                    key_row(page, glyph(key, *device), &i18n::tr(what));
                }
                Line::Custom(keyboard, pad, what) => {
                    let label = match *device {
                        InputDevice::Keyboard => keyboard,
                        InputDevice::Gamepad => pad,
                    };
                    key_row(page, &i18n::tr(label), &i18n::tr(what));
                }
                Line::Prose(text) => {
                    page.spawn(ui::face(text, ui::FONT_REGULAR, 16.0, ui::INK));
                }
                Line::Heading(text) => {
                    page.spawn(ui::face(text, ui::FONT_BOLD, 17.0, ui::VERMILION_INK));
                }
            }
        }
    });
}

/// Tecla num quadrinho de tinta, seguida do que ela faz.
fn key_row(parent: &mut ChildBuilder, key: &str, what: &str) {
    parent
        .spawn(Node {
            column_gap: Val::Px(12.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            ui::keycap(row, key);
            row.spawn(ui::face(what, ui::FONT_REGULAR, 16.0, ui::INK));
        });
}

#[cfg(test)]
pub(crate) fn init_systems_for_tests(world: &mut World) {
    fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
        let mut system = IntoSystem::into_system(system);
        system.initialize(world);
    }
    init(world, handle_book_keys);
    init(world, sync_modal);
    init(world, show_book);
    init(world, draw_tabs);
    init(world, draw_page);
    init(world, draw_footer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_retraces_the_exact_trail_then_closes() {
        let mut book = HelpBook::default();
        book.open_at(0);
        book.turn(true);
        book.turn(true);
        assert_eq!(book.page, 2);
        assert!(book.back());
        assert_eq!(book.page, 1);
        assert!(book.back());
        assert_eq!(book.page, 0);
        assert!(!book.back(), "na capa não há para onde voltar");
    }

    #[test]
    fn pages_wrap_around() {
        let mut book = HelpBook::default();
        book.open_at(0);
        book.turn(false);
        assert_eq!(book.page, PAGES.len() - 1);
    }

    #[test]
    fn every_page_but_options_has_content() {
        for page in 0..PAGES.len() {
            if page != OPTIONS_PAGE {
                assert!(!page_lines(page).is_empty(), "página {page} vazia");
            }
        }
    }
}
