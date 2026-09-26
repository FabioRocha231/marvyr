//! Tabela PT-BR → inglês das telas de porto, mercado, guilda, oficina,
//! clima e feed (MV-062). Separada de `i18n::TABLE` (HUD, livreto, guia)
//! só para as duas crescerem sem pisar uma na outra; `i18n::translate`
//! consulta as duas.

pub const TABLE: &[(&str, &str)] = &[
    // Tela de porto: abas, cabeçalho, rodapé
    ("Porto", "Port"),
    ("Porto: ?", "Port: ?"),
    ("Porão", "Hold"),
    ("Equipamento", "Loadout"),
    ("Fabricação", "Crafting"),
    ("Estaleiro", "Shipyard"),
    ("Mercado", "Market"),
    ("Guilda", "Guild"),
    ("Contratos", "Contracts"),
    ("Desatracar [ESC]", "Undock [ESC]"),
    (
        "Tab/Shift+Tab: abas · Setas: escolher · Enter: executar · ESC: desatracar",
        "Tab/Shift+Tab: tabs · Arrows: choose · Enter: confirm · ESC: undock",
    ),
    ("ERRO", "ERROR"),
    // Tela de porto: ações e linhas
    ("Depositar tudo", "Deposit all"),
    ("Retirar tudo", "Withdraw all"),
    ("Desequipar {0}", "Unequip {0}"),
    ("Equipar", "Equip"),
    ("Construir {0}", "Build {0}"),
    ("Fabricar {0}", "Craft {0}"),
    ("receita", "recipe"),
    ("Desatracar", "Undock"),
    ("Casco", "Hull"),
    ("Velas", "Sails"),
    ("Armas", "Weapons"),
    ("Auxiliar", "Auxiliary"),
    ("Qualquer estação", "Any station"),
    ("Bancada", "Workbench"),
    ("Bigorna", "Anvil"),
    ("Doca", "Dock"),
    ("Porão: {0} / {1}", "Hold: {0} / {1}"),
    ("(vazio)", "(empty)"),
    (
        "Armazém: nada compatível com este casco",
        "Warehouse: nothing fits this hull",
    ),
    (
        "Receitas de casco — custos saem do armazém do porto.",
        "Hull recipes — costs come from the port warehouse.",
    ),
    ("rende {0} x{1}", "yields {0} x{1}"),
    ("Mercado regional", "Regional market"),
    // Mercado
    ("OFERTAS DE TROCA", "TRADE OFFERS"),
    ("Mercado: sem ofertas", "Market: no offers"),
    ("OFERECER TROCA", "OFFER A TRADE"),
    ("Qtd", "Qty"),
    ("Dou", "Give"),
    ("Por", "For"),
    ("por", "for"),
    ("Criar oferta", "Post offer"),
    (
        "Sai do armazém deste porto. Quem aceita entrega o pedido inteiro.",
        "Taken from this port's warehouse. Whoever accepts hands over the full ask.",
    ),
    (
        "Clique no campo e digite · Shift+clique: ±10 · Enter envia",
        "Click a field and type · Shift+click: ±10 · Enter submits",
    ),
    ("Cancelar", "Cancel"),
    ("Trocar", "Trade"),
    ("MINHA", "MINE"),
    // Guilda e contratos
    ("GUILDA MERCANTE", "MERCHANT GUILD"),
    (
        "GUILDA MERCANTE - paga em {0}, do armazém deste porto (o item entregue é destruído)",
        "MERCHANT GUILD - pays in {0}, from this port's warehouse (items handed in are destroyed)",
    ),
    (
        "Aguardando a tabela da guilda...",
        "Waiting for the guild's rates...",
    ),
    ("Armazém", "Warehouse"),
    ("10 rendem aqui", "10 fetch here"),
    ("Trocar 1", "Trade 1"),
    ("Tudo", "All"),
    (
        "Trocar muito derruba a taxa; ela se recupera com o tempo. Deposite o porão para trocar.",
        "Trading a lot lowers the rate; it recovers over time. Deposit your hold to trade.",
    ),
    ("SEU CONTRATO", "YOUR CONTRACT"),
    ("{0} restantes - {1}", "{0} left - {1}"),
    ("abates {0}/{1}", "kills {0}/{1}"),
    (
        "entregue ao atracar no destino",
        "delivered on docking at the destination",
    ),
    ("Abandonar", "Abandon"),
    ("Nenhum contrato ativo.", "No active contract."),
    ("QUADRO DE CONTRATOS", "CONTRACT BOARD"),
    (
        "Quadro vazio - volte mais tarde.",
        "Board is empty - check back later.",
    ),
    ("Aceitar", "Accept"),
    (
        "1 contrato por vez. Entrega: a carga vai no porão e pode ser saqueada no caminho.",
        "One contract at a time. Deliveries ride in your hold and can be plundered en route.",
    ),
    (
        "CONTRATO: {0}\n{1} - {2} - {3}",
        "CONTRACT: {0}\n{1} - {2} - {3}",
    ),
    ("Escolha uma receita para ver o custo.", "Pick a recipe to see its cost."),
    // Armazém por porto
    (
        "Armazém deste porto (cada porto guarda o seu):",
        "This port's storage (each port keeps its own):",
    ),
    ("Guardado em outros portos:", "Stored in other ports:"),
    ("{0}: {1} itens", "{0}: {1} items"),
    // Clima e munição
    ("TEMPESTADE!", "STORM!"),
    ("VELAS {0}%", "SAILS {0}%"),
    ("MUNIÇÃO: BALA", "AMMO: ROUND SHOT"),
    ("MUNIÇÃO: CORRENTE", "AMMO: CHAIN SHOT"),
    (
        "Dano cheio no casco, alcance longo",
        "Full hull damage, long range",
    ),
    (
        "Rasga velas (inimigo fica lento), pouco casco, alcance curto",
        "Shreds sails (target slows down), little hull damage, short range",
    ),
    // Reputação e zonas
    ("PROCURADO", "WANTED"),
    ("PROCURADO - notoriedade {0}", "WANTED - notoriety {0}"),
    ("SUSPEITO - notoriedade {0}", "SUSPECT - notoriety {0}"),
    ("PvP desativado", "PvP off"),
    ("PvP ATIVO - full loot", "PvP ON - full loot"),
    // Lugares
    ("Porto da Serra", "Ridge Harbor"),
    ("Porto da Mina", "Mine Harbor"),
    ("Porto do Coral Negro", "Black Coral Harbor"),
    ("Porto das Gaivotas", "Gull Harbor"),
    ("Porto do Farol", "Lighthouse Harbor"),
    ("Porto da Areia Branca", "White Sand Harbor"),
    ("Porto Santa Luzia", "Santa Luzia Harbor"),
    ("Ilha do Coral Negro", "Black Coral Isle"),
    ("Águas Negras", "Black Waters"),
    ("ÁGUAS NEGRAS", "BLACK WATERS"),
    ("PASSAGEM DO SORVEDOURO", "MAELSTROM PASSAGE"),
    // Itens e recursos (nomes do catálogo do servidor)
    ("Madeira", "Timber"),
    ("Minério", "Ore"),
    ("Coral Negro", "Black Coral"),
    ("Casco Reforçado", "Reinforced Hull"),
    ("Velas de Corrida", "Racing Sails"),
    ("Canhão de Bronze", "Bronze Cannon"),
    ("Mapa do Tesouro", "Treasure Map"),
    ("Pérola Abissal", "Abyssal Pearl"),
    ("Essência da Cerração", "Fog Essence"),
    ("Âmbar Abissal", "Abyssal Amber"),
    ("Casco Negro", "Black Hull"),
    ("Velas de Cerração", "Fog Sails"),
    ("Canhões Abissais", "Abyssal Cannons"),
    ("Cristal da Cerração", "Fog Crystal"),
    ("Casco de Cristal", "Crystal Hull"),
    ("Canhões de Cristal", "Crystal Cannons"),
    // Carta náutica (MV-066)
    ("Carta Náutica", "Nautical Chart"),
    ("M fecha a carta", "M closes the chart"),
    (
        "Carta náutica: o que você já navegou",
        "Nautical chart: everywhere you have sailed",
    ),
    ("Ligar e desligar a música", "Toggle the music"),
    // Renome (MV-067)
    ("RENOME", "RENOWN"),
    ("RENOME {0}", "RENOWN {0}"),
    ("+{0} Renome", "+{0} Renown"),
    ("+{0} Renome · {1}", "+{0} Renown · {1}"),
    ("Renome e nível", "Renown and level"),
    (
        "Seu nível é o Renome do capitão. Ele sobe coletando, fabricando, construindo navio, entregando contrato, saqueando destroço e afundando navio. Cada nível dá um ponto na Rosa dos Ventos (tecla I).",
        "Your level is the captain's Renown. It grows by gathering, crafting, building ships, delivering contracts, looting wrecks and sinking ships. Each level gives one point on the Compass Rose (key I).",
    ),
    ("Nv {0} · {1}/{2}", "Lv {0} · {1}/{2}"),
    ("Nv {0} · máx", "Lv {0} · max"),
    (
        "+1 ponto na Rosa dos Ventos · tecla I",
        "+1 point in the Compass Rose · key I",
    ),
    // Rosa dos Ventos (MV-067)
    ("Rosa dos Ventos", "Compass Rose"),
    (
        "Rosa dos Ventos: talentos ganhos com Renome",
        "Compass Rose: talents earned with Renown",
    ),
    (
        "Renome {0} · {1} ponto(s) livre(s)",
        "Renown {0} · {1} free point(s)",
    ),
    (
        "Clique num nó liberado para aprender · I fecha",
        "Click an unlocked node to learn it · I closes",
    ),
    ("Redistribuir ({0} {1})", "Respec ({0} {1})"),
    ("Redistribuir só no porto", "Respec only at port"),
    ("Navegação", "Navigation"),
    ("Artilharia", "Gunnery"),
    ("Comércio", "Trade"),
    ("velocidade", "speed"),
    ("giro", "turning"),
    ("casco", "hull"),
    ("dano", "damage"),
    ("alcance", "range"),
    ("porão", "hold"),
    ("Mão no Leme", "Hand on the Helm"),
    ("Pano Bem Cazado", "Trimmed Canvas"),
    ("Costado Firme", "Firm Sides"),
    ("Rumo Certo", "True Heading"),
    ("Bolina Cerrada", "Close-Hauled"),
    ("Tábuas Dobradas", "Doubled Planks"),
    ("Timoneiro Veterano", "Veteran Helmsman"),
    ("Vento de Popa", "Following Wind"),
    ("Casco de Carvalho", "Oak Hull"),
    ("Contravento", "Upwind"),
    ("Velas Remendadas", "Patched Sails"),
    ("Pólvora Seca", "Dry Powder"),
    ("Olho de Artilheiro", "Gunner's Eye"),
    ("Bala Calibrada", "Calibrated Shot"),
    ("Alça de Mira", "Rear Sight"),
    ("Culatra Reforçada", "Reinforced Breech"),
    ("Carga Dupla", "Double Charge"),
    ("Canhão Longo", "Long Gun"),
    ("Bordada Pesada", "Heavy Broadside"),
    ("Mira Firme", "Steady Aim"),
    ("Pólvora Fina", "Fine Powder"),
    ("Olho de Mercador", "Merchant's Eye"),
    ("Estiva Arrumada", "Tidy Stowage"),
    ("Machado Afiado", "Sharp Axe"),
    ("Porão Calafetado", "Caulked Hold"),
    ("Coletor Paciente", "Patient Gatherer"),
    ("Porão Fundo", "Deep Hold"),
    ("Mãos de Ouro", "Golden Hands"),
    ("Rede de Estiva", "Cargo Net"),
    ("Faro de Coletor", "Gatherer's Nose"),
    ("Talento desconhecido.", "Unknown talent."),
    ("Talento já aprendido.", "Talent already learned."),
    (
        "Aprenda o talento anterior do ramo primeiro.",
        "Learn the previous talent in the branch first.",
    ),
    (
        "Sem pontos: ganhe Renome para subir de nível.",
        "No points: earn Renown to level up.",
    ),
    (
        "Não deu para gravar agora; tente de novo.",
        "Could not save right now; try again.",
    ),
    (
        "Redistribuir só no porto: atraque primeiro.",
        "Respec only at port: dock first.",
    ),
    ("Nenhum talento para esquecer.", "No talents to forget."),
    ("navio afundado", "ship sunk"),
    ("destroço saqueado", "wreck looted"),
    ("coleta", "gathering"),
    ("fabricação", "crafting"),
    ("navio construído", "ship built"),
    ("contrato entregue", "contract delivered"),
    ("capitão afundado", "captain sunk"),
    // Nós das fronteiras (MV-067)
    ("Madeira à Deriva", "Driftwood"),
    ("Veio Submerso", "Sunken Ore Vein"),
    // Cosméticos (MV-066)
    ("Velas Esmeralda", "Emerald Sails"),
    ("Velas de Ouro", "Golden Sails"),
    ("Velas Azul-Mar", "Sea-Blue Sails"),
    ("Estandarte de Linho", "Linen Standard"),
    ("Estandarte Esmeralda", "Emerald Standard"),
    ("Usar {0}", "Wear {0}"),
    ("Tirar {0}", "Remove {0}"),
    ("velas cosméticas", "cosmetic sails"),
    ("bandeira cosmética", "cosmetic flag"),
    ("velas do casco", "hull sails"),
    ("bandeira da casa", "house flag"),
    (
        "Visual: {0} · {1} — só aparência",
        "Look: {0} · {1} — appearance only",
    ),
    // v24: gemas de suporte
    ("Gemas", "Gems"),
    ("PEÇAS INSTALADAS", "INSTALLED PIECES"),
    ("Instale peças na aba Equipamento para ter encaixes.", "Install pieces in the Loadout tab to get sockets."),
    ("GEMAS NO ARMAZÉM", "GEMS IN STORAGE"),
    ("Nenhuma gema aqui. Lapide na aba Fabricação.", "No gems here. Cut some in the Crafting tab."),
    (
        "Arraste a gema até um encaixe · arraste para fora ou clique com o botão direito para tirar",
        "Drag a gem onto a socket · drag it out or right-click to remove",
    ),
    ("{0} encaixe(s) livre(s)", "{0} free socket(s)"),
    ("Rubi", "Ruby"),
    ("Safira", "Sapphire"),
    ("Esmeralda", "Emerald"),
    ("Topázio", "Topaz"),
    ("Ametista", "Amethyst"),
    ("Diamante", "Diamond"),
    ("Pólvora Negra", "Black Powder"),
    ("Mira Longa", "Long Sight"),
    ("Carga Rápida", "Quick Load"),
    ("Casco Selado", "Sealed Hull"),
    ("Vento Preso", "Caught Wind"),
    ("Leme Fino", "Fine Rudder"),
    ("gema encaixada", "gem socketed"),
    ("gema no armazém", "gem back in storage"),
    ("sem encaixe livre nesta peça", "no free socket on this piece"),
    ("essa gema não está no armazém deste porto", "that gem is not in this port's storage"),
    ("não há gema nesse encaixe", "no gem in that socket"),
    ("não há peça nesse slot", "no piece in that slot"),
    ("devagar com as gemas", "easy with the gems"),
    ("atraca primeiro (E) — gema é serviço de porto", "dock first (E) — gems are a port service"),
    ("o porto não conseguiu registrar; tente de novo", "the port could not record it; try again"),
    // v25: frascos de bordo
    ("Estopa", "Oakum"),
    ("Vento", "Wind"),
    ("Fúria", "Fury"),
    ("Breu", "Tar"),
    ("Frasco de Estopa", "Oakum Flask"),
    ("Frasco de Vento", "Wind Flask"),
    ("Frasco de Fúria", "Fury Flask"),
    ("Frasco de Breu", "Tar Flask"),
    ("sem esse frasco no porão", "that flask is not in the hold"),
    ("já está fazendo efeito", "already in effect"),
    ("frasco vazio: acerte canhão ou atraque", "empty flask: land hits or dock"),
    ("1 a 4", "1 to 4"),
    (
        "Frascos do porão: Estopa, Vento, Fúria e Breu. Acertos recarregam; o porto enche",
        "Flasks in the hold: Oakum, Wind, Fury and Tar. Hits recharge them; port refills",
    ),
];

#[cfg(test)]
mod tests {
    use crate::i18n::{translate, trf_in, Lang};

    #[test]
    fn port_and_item_keys_translate() {
        assert_eq!(translate("Madeira", Lang::En), "Timber");
        assert_eq!(translate("Porão", Lang::En), "Hold");
        assert_eq!(translate("MUNIÇÃO: CORRENTE", Lang::En), "AMMO: CHAIN SHOT");
        assert_eq!(
            trf_in("Fabricar {0}", &["Bronze Cannon"], Lang::En),
            "Craft Bronze Cannon"
        );
    }

    #[test]
    fn templates_keep_their_placeholders() {
        for (pt, en) in super::TABLE {
            for index in 0..4 {
                let slot = format!("{{{index}}}");
                assert_eq!(pt.contains(&slot), en.contains(&slot), "{pt}");
            }
        }
    }
}
