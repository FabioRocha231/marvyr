//! Idiomas do client (MV-062): PT-BR e inglês.
//!
//! A chave é o próprio texto em PT-BR. `tr` devolve a tradução do idioma
//! ativo; sem entrada na tabela, o PT-BR passa direto, então o jogo nunca
//! mostra uma chave crua. Modelos com `{0}`, `{1}`… passam por `trf`.
//!
//! ponytail: tabela linear e casamento exato. Mensagens dinâmicas do
//! servidor só traduzem quando batem com uma entrada; códigos de motivo no
//! protocolo quando o servidor começar a falar inglês de verdade.

use std::sync::atomic::{AtomicU8, Ordering};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    #[serde(rename = "pt-BR")]
    Pt,
    #[serde(rename = "en")]
    En,
}

impl Lang {
    pub fn toggled(self) -> Self {
        match self {
            Lang::Pt => Lang::En,
            Lang::En => Lang::Pt,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::Pt => "Português (BR)",
            Lang::En => "English",
        }
    }

    /// `pt`, `pt-BR`, `pt_BR.UTF-8` → Pt; `en*` → En.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().to_ascii_lowercase();
        if raw.starts_with("pt") {
            Some(Lang::Pt)
        } else if raw.starts_with("en") {
            Some(Lang::En)
        } else {
            None
        }
    }
}

/// Espelho global do idioma: `ui::text` traduz sem precisar do `World`.
static ACTIVE: AtomicU8 = AtomicU8::new(0);

pub fn active() -> Lang {
    match ACTIVE.load(Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::Pt,
    }
}

fn set_active(lang: Lang) {
    ACTIVE.store(lang as u8, Ordering::Relaxed);
}

/// Texto no idioma ativo.
pub fn tr(pt: &str) -> String {
    translate(pt, active())
}

/// Modelo com `{0}`, `{1}`…: traduz o modelo e depois encaixa os valores.
pub fn trf(pt: &str, args: &[&str]) -> String {
    trf_in(pt, args, active())
}

pub fn trf_in(pt: &str, args: &[&str], lang: Lang) -> String {
    let mut out = translate(pt, lang);
    for (index, arg) in args.iter().enumerate() {
        out = out.replace(&format!("{{{index}}}"), arg);
    }
    out
}

pub fn translate(pt: &str, lang: Lang) -> String {
    if lang == Lang::Pt {
        return pt.to_owned();
    }
    TABLE
        .iter()
        .chain(crate::i18n_extra::TABLE)
        .find(|(key, _)| *key == pt)
        .map(|(_, en)| (*en).to_owned())
        .unwrap_or_else(|| pt.to_owned())
}

/// Rótulo estático que acompanha a troca de idioma em tempo de jogo.
#[derive(Component, Debug, Clone)]
pub struct Translated(pub &'static str);

/// Bundle de texto que se retraduz quando o idioma muda.
pub fn label(key: &'static str, size: f32, color: Color) -> impl Bundle {
    (crate::ui::text(key, size, color), Translated(key))
}

/// Como `label`, numa face específica (`ui::FONT_BOLD`, `ui::FONT_REGULAR`…).
pub fn label_face(key: &'static str, font: Handle<Font>, size: f32, color: Color) -> impl Bundle {
    (crate::ui::face(key, font, size, color), Translated(key))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Settings {
    lang: Option<Lang>,
}

fn settings_path() -> std::path::PathBuf {
    crate::config::data_dir().join("settings.json")
}

fn load_settings() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_lang(lang: Lang) {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let settings = Settings { lang: Some(lang) };
    if let Ok(json) = serde_json::to_string(&settings) {
        let _ = std::fs::write(path, json);
    }
}

/// Idioma do boot: `--lang` → `MARVYR_LANG` → escolha salva → idioma do
/// sistema → PT-BR (o público da alpha).
pub fn boot_lang() -> Lang {
    let args: Vec<String> = std::env::args().collect();
    let cli = args
        .iter()
        .position(|arg| arg == "--lang")
        .and_then(|index| args.get(index + 1))
        .and_then(|raw| Lang::parse(raw));
    cli.or_else(|| {
        std::env::var("MARVYR_LANG")
            .ok()
            .and_then(|raw| Lang::parse(&raw))
    })
    .or(load_settings().lang)
    .or_else(|| sys_locale::get_locale().and_then(|raw| Lang::parse(&raw)))
    .unwrap_or_default()
}

/// Pedido de troca de idioma (F1, tela de login).
#[derive(Event, Debug, Clone, Copy)]
pub struct ChangeLang(pub Lang);

pub struct I18nPlugin;

impl Plugin for I18nPlugin {
    fn build(&self, app: &mut App) {
        let lang = boot_lang();
        set_active(lang);
        app.insert_resource(lang)
            .add_event::<ChangeLang>()
            .add_systems(PreUpdate, (apply_lang_change, retranslate).chain());
    }
}

fn apply_lang_change(mut requests: EventReader<ChangeLang>, mut lang: ResMut<Lang>) {
    if let Some(ChangeLang(next)) = requests.read().last().copied() {
        if *lang != next {
            set_active(next);
            *lang = next;
            save_lang(next);
            info!(lang = ?next, "idioma trocado");
        }
    }
}

fn retranslate(lang: Res<Lang>, mut labels: Query<(&Translated, &mut Text)>) {
    if !lang.is_changed() {
        return;
    }
    for (key, mut text) in &mut labels {
        let value = tr(key.0);
        if text.0 != value {
            text.0 = value;
        }
    }
}

/// PT-BR → inglês. Mantenha em ordem de tela (HUD, porto, mar, ajuda…).
const TABLE: &[(&str, &str)] = &[
    // HUD do mar
    ("CASCO", "HULL"),
    ("CARGA", "CARGO"),
    ("Mercante", "Merchant"),
    ("Patrulha", "Patrol"),
    ("Corsário", "Corsair"),
    ("PROTEGIDO", "PROTECTED"),
    ("FRONTEIRA", "FRONTIER"),
    ("SEM LEI", "LAWLESS"),
    ("PRONTO", "READY"),
    ("CANHÕES", "CANNONS"),
    ("BANDEIRA", "FLAG"),
    ("ARRIADA", "LOWERED"),
    ("IÇANDO", "HOISTING"),
    ("NEGRA", "BLACK"),
    ("BANDEIRA NEGRA", "BLACK FLAG"),
    ("Normal", "Normal"),
    ("Mágico", "Magic"),
    ("Raro", "Rare"),
    ("+{0} dano", "+{0} damage"),
    ("+{0}% alcance", "+{0}% range"),
    ("+{0} casco", "+{0} hull"),
    ("+{0}% velocidade", "+{0}% speed"),
    ("+{0} porão", "+{0} hold"),
    ("+{0}% leme", "+{0}% rudder"),
    ("-{0}% recarga", "-{0}% reload"),
    ("Raridade: {0} (trocar)", "Rarity: {0} (change)"),
    ("Raridade da oficina: {0}", "Workshop rarity: {0}"),
    ("Mágico: 1-2 afixos, o dobro dos insumos. Raro: 3-4 afixos, o triplo e Coral Negro.", "Magic: 1-2 affixes, double materials. Rare: 3-4 affixes, triple materials plus Black Coral."),
    ("Escolhida: {0}", "Selected: {0}"),
    ("sem afixos (Normal)", "no affixes (Normal)"),
    ("PEÇA RARA!", "RARE PIECE!"),
    ("PEÇA MÁGICA!", "MAGIC PIECE!"),
    ("Só equipamento sai Mágico ou Raro.", "Only equipment can be Magic or Rare."),
    ("Velas recolhidas", "Sails furled"),
    ("Meia vela", "Half sail"),
    ("Pano de cruzeiro", "Cruising sail"),
    ("Pano cheio", "Full sail"),
    ("Atracar em {0}", "Dock at {0}"),
    ("Saquear destroço", "Loot wreck"),
    ("Coletar {0}", "Gather {0}"),
    ("Cavar o tesouro", "Dig for treasure"),
    ("Abordar", "Board"),
    ("Reparar o casco", "Repair the hull"),
    ("Contratar marujos", "Hire sailors"),
    ("Ajuda", "Help"),
    ("Livreto", "Handbook"),
    ("Pano de manobra", "Maneuvering sail"),
    ("Meio pano", "Half sail"),
    ("Tripulação {0}/{1}", "Crew {0}/{1}"),
    ("Leme {0}%", "Rudder {0}%"),
    ("REPARANDO", "REPAIRING"),
    ("Cavando {0}%", "Digging {0}%"),
    ("MAPA DO TESOURO", "TREASURE MAP"),
    ("Mapa do tesouro", "Treasure map"),
    ("ÁGUAS DE RISCO", "DANGEROUS WATERS"),
    ("Seu navio, equipamentos e carga podem ser perdidos.", "Your ship, gear and cargo can be lost."),
    ("Içando a Bandeira Negra: em instantes todos são alvo, e você também", "Hoisting the Black Flag: soon everyone is a target, and so are you"),
    ("Bandeira Negra arriada", "Black Flag lowered"),
    ("Disparou há pouco: a Bandeira Negra só desce após 60 s sem tiro", "You fired recently: the Black Flag only comes down after 60 s without a shot"),
    ("Atracado não se iça a Bandeira Negra", "You can't hoist the Black Flag while docked"),
    ("Águas protegidas: a coroa não deixa içar a Bandeira Negra", "Protected waters: the crown forbids the Black Flag"),
    ("Águas protegidas: a coroa mandou arriar a Bandeira Negra", "Protected waters: the crown had the Black Flag lowered"),
    ("Alvo travado: o tiro automático vai nele", "Target locked: auto-fire goes for it"),
    ("Alvo solto", "Target released"),
    ("Nenhum navio no alcance para travar", "No ship in range to lock"),
    ("porto", "port"),
    ("recurso", "resource"),
    // Rumos (bússola do mapa)
    ("L", "E"),
    ("O", "W"),
    ("NO", "NW"),
    ("SO", "SW"),
    ("leste", "east"),
    ("nordeste", "northeast"),
    ("norte", "north"),
    ("noroeste", "northwest"),
    ("oeste", "west"),
    ("sudoeste", "southwest"),
    ("sul", "south"),
    ("sudeste", "southeast"),
    // Portais
    ("Cerração", "Fog Bank"),
    ("Saída da Cerração", "Fog Bank Exit"),
    ("Saída", "Exit"),
    ("Sorvedouro", "Maelstrom"),
    ("{0} vaga(s)", "{0} slot(s)"),
    // Livreto
    ("LIVRETO DO MARUJO", "SAILOR'S HANDBOOK"),
    ("Navegar", "Sailing"),
    ("Combate", "Combat"),
    ("Porto e comércio", "Port & trade"),
    ("O mar", "The sea"),
    ("Controle", "Gamepad"),
    ("Opções", "Options"),
    ("Setas ou Tab: páginas   ·   Esc: voltar   ·   F1: fechar   ·   L: idioma", "Arrows or Tab: pages   ·   Esc: back   ·   F1: close   ·   L: language"),
    ("Direcional ou LB/RB: páginas   ·   B: voltar   ·   Start: fechar   ·   Select: idioma", "D-pad or LB/RB: pages   ·   B: back   ·   Start: close   ·   Select: language"),
    ("Idioma: {0}", "Language: {0}"),
    ("Rever o guia da primeira viagem", "Replay the first-voyage guide"),
    ("Sair do jogo", "Quit game"),
    ("Setas escolhem · Enter confirma", "Arrows choose · Enter confirms"),
    ("Direcional escolhe · A confirma", "D-pad chooses · A confirms"),
    ("Içar vela (três níveis: meia, cruzeiro, cheia)", "Raise sail (three levels)"),
    ("Recolher vela", "Lower sail"),
    ("Leme: virar a bombordo e a boreste", "Rudder: turn to port and starboard"),
    ("Roda do mouse", "Mouse wheel"),
    ("Analógico direito", "Right stick"),
    ("Aproximar e afastar a câmera", "Zoom the camera in and out"),
    ("Atracar quando o bilhete de porto aparecer", "Dock when the port slip appears"),
    ("O painel no canto direito mostra suas velas e a munição. Tempestade rasga o pano e o navio fica lento; no mar ele se remenda devagar, atracado na hora.", "The panel in the right corner shows your sails and ammunition. Storms shred the sail and slow the ship; at sea it mends slowly, docked at once."),
    ("Travar o alvo do tiro automático (ou soltar)", "Lock the auto-fire target (or release)"),
    ("Içar ou arriar a Bandeira Negra", "Hoist or lower the Black Flag"),
    ("Trocar a munição", "Switch ammunition"),
    ("Reparar no mar: gasta madeira, só parado e fora de combate", "Repair at sea: uses timber, only when stopped and out of combat"),
    ("Abordar um navio avariado, lado a lado e devagar", "Board a crippled ship, alongside and slow"),
    ("Saquear um destroço (passar por cima já recolhe)", "Loot a wreck (sailing over it picks it up)"),
    ("Coletar no ponto de recurso marcado no mar", "Gather at a resource point marked on the sea"),
    ("Atracar e desatracar", "Dock and undock"),
    ("Trocar de aba no porto (porão, mercado, fabricação…)", "Switch port tabs (hold, market, crafting…)"),
    ("Confirmar a linha escolhida", "Confirm the selected row"),
    ("Contratar marujos (atracado, paga em Madeira)", "Hire sailors (while docked, paid in Timber)"),
    ("Não há moeda: tudo se troca. Tudo que vale algo é fabricado por jogadores. Colete, fabrique, carregue o porão e troque onde o seu recurso vale mais. O caminho entre os portos é o risco — e o lucro.", "There is no coin: everything is bartered. Everything of value is made by players. Gather, craft, fill the hold and trade where your goods are worth most. The route between ports is the risk — and the profit."),
    ("Zonas", "Zones"),
    ("Águas protegidas: ninguém ataca você. Fronteira: combate liberado. Sem lei: combate e saque total — afundou, a carga vira destroço de quem pegar.", "Protected waters: nobody can attack you. Frontier: combat allowed. Lawless: combat and full loot — sink, and your cargo becomes a wreck for whoever gets there."),
    ("Portais", "Portals"),
    ("Cerração: banco de névoa com tempo e vagas contados; leva a uma arena isolada, diferente a cada abertura, com baús de Cristal da Cerração que afundam em 5 minutos. Devolve você quando se dissipa. Sorvedouro: redemoinho que liga pontos distantes por dentro de águas sem lei.", "Fog Bank: a timed fog with limited slots; it leads to an isolated arena, different every time, with Fog Crystal chests that sink in 5 minutes. It returns you when it lifts. Maelstrom: a whirlpool linking distant points through lawless water."),
    ("Eventos de mar", "Sea events"),
    ("Tormenta desgasta o casco de quem está dentro. Frota do tesouro navega com escolta. O kraken morde quem chega perto. Maré disputada faz brotar recurso raro em mar aberto.", "A tempest wears down every hull inside it. The treasure fleet sails with an escort. The kraken bites anyone who comes close. A contested tide spawns rare resources in open water."),
    ("Tesouro", "Treasure"),
    ("Às vezes a coleta rende um mapa. Leve-o até o X marcado no mar, pare o navio e cave.", "Gathering sometimes turns up a map. Take it to the X marked on the sea, stop the ship and dig."),
    ("Direcional cima e baixo", "D-pad up and down"),
    ("Analógico esquerdo", "Left stick"),
    ("Travar alvo e Bandeira Negra", "Lock target and Black Flag"),
    ("D-pad cima", "D-pad up"),
    ("D-pad baixo", "D-pad down"),
    ("Analógico", "Stick"),
    ("Desatraque para zarpar", "Undock to set sail"),
    ("Ação do bilhete: atracar, coletar, saquear, abordar, cavar", "Slip action: dock, gather, loot, board, dig"),
    ("Reparar", "Repair"),
    ("Munição", "Ammunition"),
    ("Este livreto", "This handbook"),
    ("Idioma", "Language"),
    ("No porto: setas ou direcional escolhem, Enter ou A confirma, Tab ou LB/RB troca de aba, Esc ou B volta.", "In port: arrows or D-pad choose, Enter or A confirms, Tab or LB/RB switches tab, Esc or B goes back."),
    // Primeira viagem
    ("BEM-VINDO A BORDO", "WELCOME ABOARD"),
    ("Você é mercador. Tudo que vale algo neste mar foi fabricado por um jogador.", "You are a merchant. Everything of value on this sea was made by a player."),
    ("Riqueza só vale onde ela chega: carregue o porão e leve até o porto que paga mais.", "Wealth is only worth something where it arrives: fill the hold and carry it to the port that pays most."),
    ("No caminho, o mar cobra. Em águas sem lei, quem afunda perde a carga.", "On the way, the sea takes its toll. In lawless waters, whoever sinks loses the cargo."),
    ("O guia da primeira viagem tem seis passos, uns dez minutos.", "The first-voyage guide has six steps, about ten minutes."),
    ("Zarpar com o guia", "Set sail with the guide"),
    ("Já sei navegar", "I know how to sail"),
    ("F1 abre o livreto do marujo a qualquer hora.", "F1 opens the sailor's handbook at any time."),
    ("Start abre o livreto do marujo a qualquer hora.", "Start opens the sailor's handbook at any time."),
    ("PRIMEIRA VIAGEM · {0} DE {1}", "FIRST VOYAGE · {0} OF {1}"),
    ("FEITO", "DONE"),
    ("Içe a vela para sair do porto", "Raise the sail to leave port"),
    ("Colete um recurso no mar", "Gather a resource at sea"),
    ("Volte a um porto e atraque", "Return to a port and dock"),
    ("Guarde ou troque a carga no porto", "Store or trade the cargo in port"),
    ("Fabrique algo no porto", "Craft something in port"),
    ("Leve carga até outro porto", "Carry cargo to another port"),
    ("{0} a {1}, {2}", "{0}, {1} {2}"),
    ("W iça as velas, A e D viram o leme. Quanto mais pano, mais rápido.", "W raises the sails, A and D turn the rudder. More sail, more speed."),
    ("Pontos de recurso aparecem no mar com o nome e o estoque.", "Resource points show on the sea with name and stock."),
    ("Chegue perto do porto até o bilhete de atracar aparecer.", "Get close to the port until the docking slip appears."),
    ("Aba Porão: Depositar tudo guarda a carga. Mercado e Guilda: trocar.", "Hold tab: Deposit all stores the cargo. Market and Guild: trade."),
    ("Aba Fabricação: escolha uma receita com os materiais que você tem.", "Crafting tab: pick a recipe you have the materials for."),
    ("Cada porto paga diferente: o lucro está na viagem.", "Every port pays differently: the profit is in the voyage."),
    ("VOCÊ É MERCADOR", "YOU ARE A MERCHANT"),
    ("Agora é com você: leve o que sobra num porto para onde falta e escolha os seus riscos.", "Now it's up to you: carry what one port has plenty of to where it's scarce, and choose your risks."),
    ("Fronteira", "Frontier"),
    ("Daqui em diante outros capitães podem atacar você. Carga valiosa pede escolta ou pressa.", "From here on other captains can attack you. Valuable cargo calls for an escort or speed."),
    ("Águas sem lei", "Lawless waters"),
    ("Combate e saque total: afundou, a carga vira destroço de quem pegar. O recurso raro está aqui — e o risco também.", "Combat and full loot: sink, and your cargo becomes a wreck for whoever gets there. The rare resources are here — and so is the risk."),
    ("Cerração e Sorvedouro", "Fog Banks and Maelstroms"),
    ("Cerração é névoa com tempo e vagas contados que leva a uma arena. Sorvedouro é um redemoinho que corta caminho por águas sem lei.", "A Fog Bank is timed fog with limited slots that leads to an arena. A Maelstrom is a whirlpool that cuts through lawless water."),
    ("Evento de mar", "Sea event"),
    ("Tormenta, frota do tesouro, kraken ou maré disputada. O anúncio diz onde; o livreto (F1) conta o que cada um faz.", "Tempest, treasure fleet, kraken or contested tide. The notice says where; the handbook (F1) explains each one."),
    ("Siga o X marcado no mar, pare o navio em cima e cave.", "Follow the X marked on the sea, stop the ship on it and dig."),
    ("Casco avariado", "Damaged hull"),
    ("Parado e fora de combate, repare no mar gastando madeira do porão.", "Stopped and out of combat, repair at sea using timber from the hold."),
    ("Porão cheio", "Hold full"),
    ("Atraque para guardar no armazém do porto ou trocar no mercado.", "Dock to store in the port warehouse or trade at the market."),
    // Entrada
    ("O transporte arriscado de riqueza fabricada por jogadores", "The risky transport of player-made wealth"),
    ("Capitão", "Captain"),
    ("Senha", "Password"),
    ("Lembrar de mim", "Remember me"),
    ("Entrar", "Log in"),
    ("Criar conta", "Create account"),
    ("Tab troca de campo · nome: 3 a 20 letras, números ou _ · senha: 8+", "Tab switches field · name: 3 to 20 letters, digits or _ · password: 8+"),
    ("Enter tenta de novo  ·  Esc sai", "Enter tries again  ·  Esc quits"),
    ("Esc sai", "Esc quits"),
    ("Verificando a conta…", "Checking your account…"),
    ("Conectando a {0}…", "Connecting to {0}…"),
    ("Conexão perdida.\nSeu navio fica 60 s no mar antes de ancorar no porto.", "Connection lost.\nYour ship stays at sea for 60 s before anchoring in port."),
    ("Servidor indisponível ({0}).", "Server unavailable ({0})."),
    ("Esta versão do Marvyr está desatualizada.\n\nAtualize o jogo pelo itch.io.", "This version of Marvyr is out of date.\n\nUpdate the game on itch.io."),
    ("Conexão recusada.", "Connection refused."),
    ("Servidor do Marvyr não configurado.", "Marvyr server not configured."),
    ("Crie marvyr.toml ao lado do executável do jogo com:", "Create marvyr.toml next to the game executable with:"),
    ("Sem resposta em 10 s. Confira a internet ou tente mais tarde.", "No answer in 10 s. Check your internet or try again later."),
    ("Nenhum servidor foi informado nesta build.", "No server was set in this build."),
    ("Sessão ausente. Faça login novamente.", "No session. Please log in again."),
    ("Sessão expirada ou inválida. Faça login novamente.", "Session expired or invalid. Please log in again."),
    ("Este servidor exige login com uma conta Marvyr.", "This server requires a Marvyr account login."),
    ("Servidor cheio. Tente de novo em alguns minutos.", "Server full. Try again in a few minutes."),
    ("Seu capitão já está conectado em outra sessão. Feche o outro jogo e tente de novo.", "Your captain is already connected in another session. Close the other game and try again."),
    ("Desconectado por excesso de mensagens.", "Disconnected for sending too many messages."),
    // Águas e eventos de mar (nomes do servidor)
    ("Rota da Costa", "Coast Route"),
    ("Corredor do Amanhecer", "Dawn Passage"),
    ("Corredor do Poente", "Sunset Passage"),
    ("Mar Sem Lei", "Lawless Sea"),
    // Zonas do mundo em grafo (MV-066)
    ("Baía da Serra", "Ridge Bay"),
    ("Baía da Mina", "Mine Bay"),
    ("Mar do Coral Negro", "Black Coral Sea"),
    ("Enseada das Gaivotas", "Gull Cove"),
    ("Costa do Farol", "Lighthouse Coast"),
    ("Baixios da Areia Branca", "White Sand Shoals"),
    ("Mar de Santa Luzia", "Santa Luzia Sea"),
    ("Passagem do Sorvedouro", "Maelstrom Passage"),
    ("Tormenta", "Tempest"),
    ("Frota do Tesouro", "Treasure Fleet"),
    ("Maré de Pérolas", "Pearl Tide"),
    // Formulário e serviço de contas
    ("Nome de capitão: 3 a 20 letras, números ou _.", "Captain name: 3 to 20 letters, digits or _."),
    ("A senha precisa ter de 8 a 128 caracteres.", "Password must be 8 to 128 characters."),
    ("usuário ou senha inválidos", "invalid username or password"),
    ("nome de usuário já está em uso", "username is already taken"),
    ("muitas tentativas; aguarde um minuto", "too many attempts; wait a minute"),
    ("nome de usuário deve ter entre 3 e 20 caracteres", "username must be 3 to 20 characters"),
    ("nome de usuário aceita apenas letras, números e _", "username accepts only letters, digits and _"),
    ("senha deve ter entre 8 e 128 caracteres", "password must be 8 to 128 characters"),
    ("banco indisponível", "database unavailable"),
    ("erro interno", "internal error"),
    ("Resposta inválida do servidor de contas.", "Invalid response from the account server."),
    ("O servidor de contas recusou o pedido.", "The account server refused the request."),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_entries_fall_back_to_portuguese() {
        assert_eq!(translate("CASCO", Lang::En), "HULL");
        assert_eq!(translate("CASCO", Lang::Pt), "CASCO");
        assert_eq!(
            translate("frase sem tradução", Lang::En),
            "frase sem tradução"
        );
    }

    #[test]
    fn templates_fill_arguments_after_translating() {
        assert_eq!(
            trf_in("Coletar {0}", &["Timber"], Lang::En),
            "Gather Timber"
        );
        assert_eq!(
            trf_in("Coletar {0}", &["Madeira"], Lang::Pt),
            "Coletar Madeira"
        );
    }

    #[test]
    fn locale_strings_parse() {
        assert_eq!(Lang::parse("pt_BR.UTF-8"), Some(Lang::Pt));
        assert_eq!(Lang::parse("en-US"), Some(Lang::En));
        assert_eq!(Lang::parse("fr"), None);
    }

    #[test]
    fn table_has_no_duplicate_keys() {
        let mut keys: Vec<&str> = TABLE
            .iter()
            .chain(crate::i18n_extra::TABLE)
            .map(|(key, _)| *key)
            .collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "chave repetida na tabela");
    }
}
