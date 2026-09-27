//! v61: abordagem como duelo de táticas (pedra-papel-tesoura no convés).
//! Três rodadas curtas; em cada uma os dois lados escolhem Assalto,
//! Mosquete ou Muralha. Assalto atropela o Mosquete, o Mosquete fura a
//! Muralha, a Muralha segura o Assalto. Quem vence a rodada derruba marujos
//! do outro (mais gente, golpe maior). Rende quem perder mais rodadas ou
//! ficar sem ninguém.

/// Rodadas do duelo e segundos para escolher em cada uma.
pub const ROUNDS: u8 = 3;
pub const ROUND_SECS: f32 = 4.0;
/// Chance (em 1000) de o NPC cumprir o que anunciou; no resto ele blefa e
/// joga o que vence a resposta óbvia.
pub const NPC_HONESTY: u32 = 700;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tactic {
    Charge,
    Volley,
    Hold,
}

impl Tactic {
    pub const ALL: [Tactic; 3] = [Tactic::Charge, Tactic::Volley, Tactic::Hold];

    pub fn code(self) -> u8 {
        match self {
            Tactic::Charge => 1,
            Tactic::Volley => 2,
            Tactic::Hold => 3,
        }
    }

    pub fn from_code(code: u8) -> Option<Tactic> {
        Tactic::ALL.into_iter().find(|tactic| tactic.code() == code)
    }

    pub fn name(self) -> &'static str {
        match self {
            Tactic::Charge => "Assalto",
            Tactic::Volley => "Mosquete",
            Tactic::Hold => "Muralha",
        }
    }

    /// A tática que vence esta.
    pub fn counter(self) -> Tactic {
        match self {
            Tactic::Charge => Tactic::Hold,
            Tactic::Volley => Tactic::Charge,
            Tactic::Hold => Tactic::Volley,
        }
    }

    pub fn beats(self, other: Tactic) -> bool {
        other.counter() == self
    }
}

/// Intenção anunciada e jogada real do NPC a partir de um rolo.
pub fn npc_round(roll: u32) -> (Tactic, Tactic) {
    let intent = Tactic::ALL[(roll % 3) as usize];
    let honest = (roll / 3) % 1000 < NPC_HONESTY;
    // Blefe: espera que você responda o anúncio e joga o que vence isso.
    let played = if honest {
        intent
    } else {
        intent.counter().counter()
    };
    (intent, played)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundResult {
    Attacker,
    Defender,
    Draw,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Melee {
    /// Rodada atual (1..=ROUNDS).
    pub round: u8,
    pub attacker_crew: u16,
    pub defender_crew: u16,
    /// Casco do defensor (0..1): avariado, luta pior.
    defender_hull: f32,
    pub attacker_wins: u8,
    pub defender_wins: u8,
    pub attacker_losses: u16,
    pub defender_losses: u16,
}

impl Melee {
    pub fn new(attacker_crew: u16, defender_crew: u16, defender_hull: f32) -> Self {
        Self {
            round: 1,
            attacker_crew,
            defender_crew,
            defender_hull: defender_hull.clamp(0.0, 1.0),
            attacker_wins: 0,
            defender_wins: 0,
            attacker_losses: 0,
            defender_losses: 0,
        }
    }

    /// Resolve a rodada com as escolhas (sem escolha = Muralha) e devolve
    /// o resultado e as baixas (atacante, defensor).
    pub fn resolve(&mut self, attack: Tactic, defend: Tactic) -> (RoundResult, u16, u16) {
        let result = if attack.beats(defend) {
            RoundResult::Attacker
        } else if defend.beats(attack) {
            RoundResult::Defender
        } else {
            RoundResult::Draw
        };
        let hit = |crew: u16, pct: f32| (f32::from(crew) * pct).ceil() as u16;
        let defense = 0.5 + 0.5 * self.defender_hull;
        let (a_loss, d_loss) = match result {
            RoundResult::Attacker => (0, hit(self.attacker_crew, 0.35)),
            RoundResult::Defender => (
                hit((f32::from(self.defender_crew) * defense) as u16, 0.35),
                0,
            ),
            RoundResult::Draw => (
                hit(self.defender_crew, 0.1 * defense),
                hit(self.attacker_crew, 0.1),
            ),
        };
        let a_loss = a_loss.min(self.attacker_crew);
        let d_loss = d_loss.min(self.defender_crew);
        self.attacker_crew -= a_loss;
        self.defender_crew -= d_loss;
        self.attacker_losses += a_loss;
        self.defender_losses += d_loss;
        match result {
            RoundResult::Attacker => self.attacker_wins += 1,
            RoundResult::Defender => self.defender_wins += 1,
            RoundResult::Draw => {}
        }
        self.round += 1;
        (result, a_loss, d_loss)
    }

    /// `Some(true)` = o navio rendeu; `Some(false)` = repelida; `None` =
    /// ainda luta.
    pub fn outcome(&self) -> Option<bool> {
        if self.defender_crew == 0 && self.attacker_crew > 0 {
            return Some(true);
        }
        if self.attacker_crew == 0 {
            return Some(false);
        }
        let left = ROUNDS.saturating_sub(self.round - 1);
        let (a, d) = (self.attacker_wins, self.defender_wins);
        if a > d + left {
            Some(true)
        } else if a + left <= d {
            // Nem vencendo o resto passa à frente (empate fica com quem
            // defende).
            Some(false)
        } else if left == 0 {
            Some(a > d)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tactic_beats_exactly_one() {
        for tactic in Tactic::ALL {
            let beaten = Tactic::ALL
                .iter()
                .filter(|other| tactic.beats(**other))
                .count();
            assert_eq!(beaten, 1, "{}", tactic.name());
            assert!(tactic.counter().beats(tactic));
            assert_eq!(Tactic::from_code(tactic.code()), Some(tactic));
        }
        assert!(Tactic::Charge.beats(Tactic::Volley));
        assert!(Tactic::Volley.beats(Tactic::Hold));
        assert!(Tactic::Hold.beats(Tactic::Charge));
    }

    #[test]
    fn npc_bluffs_by_beating_the_obvious_answer() {
        let honest = npc_round(0);
        assert_eq!(honest.0, honest.1);
        // Rolo fora da honestidade: joga o que vence a resposta ao anúncio.
        let bluff = npc_round(3 * 999);
        assert!(bluff.1.beats(bluff.0.counter()));
    }

    #[test]
    fn two_round_wins_take_the_ship_and_a_final_tie_holds() {
        let mut melee = Melee::new(12, 10, 0.3);
        melee.resolve(Tactic::Charge, Tactic::Volley);
        assert_eq!(melee.outcome(), None);
        melee.resolve(Tactic::Volley, Tactic::Hold);
        assert_eq!(melee.outcome(), Some(true), "2 de 3 já decide");
        assert!(melee.defender_losses > 0 && melee.attacker_losses == 0);

        let mut tied = Melee::new(8, 8, 1.0);
        tied.resolve(Tactic::Hold, Tactic::Hold);
        tied.resolve(Tactic::Charge, Tactic::Hold);
        tied.resolve(Tactic::Charge, Tactic::Volley);
        assert_eq!(
            tied.outcome(),
            Some(false),
            "1 a 1 no fim: segura quem defende"
        );
    }

    #[test]
    fn empty_deck_ends_the_fight() {
        let mut melee = Melee::new(2, 1, 0.1);
        melee.resolve(Tactic::Charge, Tactic::Volley);
        assert_eq!(melee.defender_crew, 0);
        assert_eq!(melee.outcome(), Some(true));
    }
}
