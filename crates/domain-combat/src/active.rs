//! Combate ativo (v54): o capitão mira com o mouse, solta a salva em leque,
//! larga barril incendiário na esteira e abalroa. Regras puras — o servidor
//! decide quem pode e aplica o dano pelo mesmo caminho das balas.

use serde::{Deserialize, Serialize};

use marvyr_domain_items::GemKind;

use crate::projectile::{Projectile, ProjectileKind, WeaponParams};

/// Balas da salva em leque e a abertura total do leque (radianos, ~50°).
pub const FAN_BALLS: u32 = 7;
pub const FAN_SPREAD: f32 = 0.87;
/// Dano total do leque em múltiplos do dano da arma (repartido entre as
/// balas): de perto, quase todas acertam e o golpe é pesado.
pub const FAN_DAMAGE: f32 = 2.2;
pub const FAN_COOLDOWN_SECS: f32 = 8.0;

/// Barril incendiário: boia parado na esteira e explode em quem passa.
pub const BARREL_DAMAGE: f32 = 2.5;
pub const BARREL_COOLDOWN_SECS: f32 = 10.0;
pub const BARREL_LIFETIME_SECS: f32 = 20.0;

/// Abalroar: arrancada curta que também serve de esquiva.
pub const RAM_COOLDOWN_SECS: f32 = 7.0;
pub const RAM_SECS: f32 = 1.0;
/// Velocidade da arrancada em múltiplos da velocidade máxima.
pub const RAM_BOOST: f32 = 1.9;
/// Dano do choque em múltiplos do dano da arma.
pub const RAM_DAMAGE: f32 = 2.0;
/// Alcance do choque em múltiplos do raio de colisão do casco.
pub const RAM_REACH: f32 = 2.2;

/// v57: Abalroar Longo (Ametista): arrancada maior, recarga menor.
pub const LONG_RAM_SECS: f32 = 1.8;
pub const LONG_RAM_COOLDOWN_SECS: f32 = 5.0;
/// v57: Tiro de Precisão (Safira): uma bala pesada, rápida e longa.
pub const PRECISION_DAMAGE: f32 = 3.5;
pub const PRECISION_SPEED: f32 = 1.6;
pub const PRECISION_RANGE: f32 = 1.3;
/// v57: Salva Incendiária (Rubi): o leque bate mais forte.
pub const INCENDIARY_BOOST: f32 = 1.5;
/// v57: Barril Rasga-Vela (Diamante): pouco casco, muito pano.
pub const SAILSHRED_HULL: f32 = 0.6;
pub const SAILSHRED_SAIL: f32 = 4.0;

/// Segundos que o tiro automático fica calado depois de um tiro na mão.
pub const MANUAL_HOLD_SECS: f32 = 4.0;
/// O tiro automático é o modo passivo (mercador que só quer passar): bate
/// menos que o capitão que mira.
pub const AUTO_DAMAGE: f32 = 0.6;

/// Uma ação de combate do capitão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CombatActionKind {
    /// Salva do bordo que encara a mira (recarga dos canhões).
    Broadside,
    /// Abalroar (arrancada + choque).
    Ram,
    /// Salva em leque na direção da mira.
    FanSalvo,
    /// Barril incendiário na esteira.
    FireBarrel,
}

/// v57: o que a skill Z vira com a gema encaixada.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FanVariant {
    #[default]
    Fan,
    /// Safira.
    Precision,
    /// Rubi.
    Incendiary,
    /// Esmeralda.
    DoubleFan,
}

/// v57: o que a skill X vira com a gema encaixada.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BarrelVariant {
    #[default]
    Barrel,
    /// Topázio.
    Triple,
    /// Diamante.
    SailShred,
}

/// v57: variantes de skill que as gemas encaixadas no navio dão.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkillVariants {
    pub fan: FanVariant,
    pub barrel: BarrelVariant,
    /// Ametista: Abalroar Longo.
    pub long_ram: bool,
}

impl SkillVariants {
    /// Gemas de todas as peças instaladas; vale a primeira da prioridade
    /// (Safira > Rubi > Esmeralda no Z, Topázio > Diamante no X).
    pub fn from_gems(gems: impl IntoIterator<Item = GemKind>) -> Self {
        let gems: Vec<GemKind> = gems.into_iter().collect();
        let has = |gem| gems.contains(&gem);
        Self {
            fan: if has(GemKind::Sapphire) {
                FanVariant::Precision
            } else if has(GemKind::Ruby) {
                FanVariant::Incendiary
            } else if has(GemKind::Emerald) {
                FanVariant::DoubleFan
            } else {
                FanVariant::Fan
            },
            barrel: if has(GemKind::Topaz) {
                BarrelVariant::Triple
            } else if has(GemKind::Diamond) {
                BarrelVariant::SailShred
            } else {
                BarrelVariant::Barrel
            },
            long_ram: has(GemKind::Amethyst),
        }
    }

    /// Códigos para o `ShipState` (leque, barril, abalroar).
    pub fn wire(self) -> [u8; 3] {
        [self.fan as u8, self.barrel as u8, u8::from(self.long_ram)]
    }
}

/// Recargas e estados das ações do capitão. Não persiste: nasce pronto.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActiveCombat {
    pub fan_cooldown: f32,
    pub barrel_cooldown: f32,
    pub ram_cooldown: f32,
    /// Segundos restantes da arrancada (0 = não está abalroando).
    pub ram_secs: f32,
    /// Navios já atingidos nesta arrancada (um choque por alvo).
    ram_hits: Vec<u32>,
    /// Segundos até o tiro automático voltar a atirar sozinho.
    pub manual_secs: f32,
}

impl ActiveCombat {
    pub fn tick(&mut self, dt: f32) {
        self.fan_cooldown = (self.fan_cooldown - dt).max(0.0);
        self.barrel_cooldown = (self.barrel_cooldown - dt).max(0.0);
        self.ram_cooldown = (self.ram_cooldown - dt).max(0.0);
        self.manual_secs = (self.manual_secs - dt).max(0.0);
        if self.ram_secs > 0.0 {
            self.ram_secs = (self.ram_secs - dt).max(0.0);
            if self.ram_secs == 0.0 {
                self.ram_hits.clear();
            }
        }
    }

    /// O capitão atirou na mão: o automático cala por um tempo.
    pub fn mark_manual(&mut self) {
        self.manual_secs = MANUAL_HOLD_SECS;
    }

    pub fn auto_silenced(&self) -> bool {
        self.manual_secs > 0.0
    }

    pub fn try_fan(&mut self) -> bool {
        start(&mut self.fan_cooldown, FAN_COOLDOWN_SECS)
    }

    pub fn try_barrel(&mut self) -> bool {
        start(&mut self.barrel_cooldown, BARREL_COOLDOWN_SECS)
    }

    pub fn try_ram(&mut self, long: bool) -> bool {
        let (secs, cooldown) = if long {
            (LONG_RAM_SECS, LONG_RAM_COOLDOWN_SECS)
        } else {
            (RAM_SECS, RAM_COOLDOWN_SECS)
        };
        if !start(&mut self.ram_cooldown, cooldown) {
            return false;
        }
        self.ram_secs = secs;
        self.ram_hits.clear();
        true
    }

    pub fn is_ramming(&self) -> bool {
        self.ram_secs > 0.0
    }

    /// `true` na primeira vez que a arrancada encosta em `ship_id`.
    pub fn ram_hit(&mut self, ship_id: u32) -> bool {
        if !self.is_ramming() || self.ram_hits.contains(&ship_id) {
            return false;
        }
        self.ram_hits.push(ship_id);
        true
    }

    /// Recargas das duas skills para o HUD (leque, barril).
    pub fn skill_cooldowns(&self) -> [f32; 2] {
        [self.fan_cooldown, self.barrel_cooldown]
    }
}

fn start(cooldown: &mut f32, secs: f32) -> bool {
    if *cooldown > 0.0 {
        return false;
    }
    *cooldown = secs;
    true
}

/// Arma com o dano multiplicado (arredondado, nunca zero se havia dano).
pub fn scaled(weapon: WeaponParams, factor: f32) -> WeaponParams {
    let damage = (weapon.damage as f32 * factor).round() as u32;
    WeaponParams {
        damage: if weapon.damage > 0 { damage.max(1) } else { 0 },
        ..weapon
    }
}

/// Salva em leque: `FAN_BALLS` balas abertas em `FAN_SPREAD` em volta de
/// `bearing`, herdando a velocidade do navio. O dano total é
/// `FAN_DAMAGE` × arma, repartido (sobra na bala do meio).
pub fn fan_salvo(
    first_id: u32,
    owner_ship_id: u32,
    ship: (f32, f32),
    ship_velocity: (f32, f32),
    bearing: f32,
    weapon: WeaponParams,
) -> Vec<Projectile> {
    let total = scaled(weapon, FAN_DAMAGE);
    let per_ball = WeaponParams {
        damage: total.damage / FAN_BALLS,
        ..weapon
    };
    let remainder = total.damage % FAN_BALLS;
    let step = FAN_SPREAD / (FAN_BALLS - 1) as f32;
    (0..FAN_BALLS)
        .map(|i| {
            let direction = bearing - FAN_SPREAD / 2.0 + step * i as f32;
            let mut ball = Projectile::aimed(
                first_id + i,
                owner_ship_id,
                ship.0,
                ship.1,
                direction,
                if i == FAN_BALLS / 2 {
                    WeaponParams {
                        damage: per_ball.damage + remainder,
                        ..per_ball
                    }
                } else {
                    per_ball
                },
            );
            let vx = direction.cos() * ball.speed + ship_velocity.0;
            let vy = direction.sin() * ball.speed + ship_velocity.1;
            ball.heading = vy.atan2(vx).rem_euclid(std::f32::consts::TAU);
            ball.speed = vx.hypot(vy);
            ball.remaining_lifetime = weapon.range / ball.speed.max(1.0);
            ball
        })
        .collect()
}

/// v57: a skill Z conforme a variante (ids a partir de `first_id`).
pub fn fan_skill(
    variant: FanVariant,
    first_id: u32,
    owner_ship_id: u32,
    ship: (f32, f32),
    ship_velocity: (f32, f32),
    bearing: f32,
    weapon: WeaponParams,
) -> Vec<Projectile> {
    match variant {
        FanVariant::Fan => fan_salvo(
            first_id,
            owner_ship_id,
            ship,
            ship_velocity,
            bearing,
            weapon,
        ),
        FanVariant::Precision => {
            let heavy = WeaponParams {
                speed: weapon.speed * PRECISION_SPEED,
                range: weapon.range * PRECISION_RANGE,
                ..scaled(weapon, PRECISION_DAMAGE)
            };
            vec![Projectile::aimed(
                first_id,
                owner_ship_id,
                ship.0,
                ship.1,
                bearing,
                heavy,
            )]
        }
        FanVariant::Incendiary => {
            let mut balls = fan_salvo(
                first_id,
                owner_ship_id,
                ship,
                ship_velocity,
                bearing,
                scaled(weapon, INCENDIARY_BOOST),
            );
            for ball in &mut balls {
                ball.kind = ProjectileKind::Fire;
            }
            balls
        }
        FanVariant::DoubleFan => {
            let mut balls = fan_salvo(
                first_id,
                owner_ship_id,
                ship,
                ship_velocity,
                bearing,
                weapon,
            );
            balls.extend(fan_salvo(
                first_id + FAN_BALLS,
                owner_ship_id,
                ship,
                ship_velocity,
                bearing + std::f32::consts::PI,
                weapon,
            ));
            balls
        }
    }
}

/// Quantos ids a skill Z gasta (o servidor reserva antes de montar).
pub fn fan_skill_ids(variant: FanVariant) -> u32 {
    match variant {
        FanVariant::Precision => 1,
        FanVariant::DoubleFan => FAN_BALLS * 2,
        _ => FAN_BALLS,
    }
}

/// v57: a skill X conforme a variante (ids a partir de `first_id`).
pub fn barrel_skill(
    variant: BarrelVariant,
    first_id: u32,
    owner_ship_id: u32,
    ship: (f32, f32),
    heading: f32,
    weapon: WeaponParams,
) -> Vec<Projectile> {
    match variant {
        BarrelVariant::Barrel => vec![barrel(first_id, owner_ship_id, ship, heading, weapon)],
        BarrelVariant::Triple => (0..3)
            .map(|i| {
                barrel(
                    first_id + i,
                    owner_ship_id,
                    ship,
                    heading,
                    WeaponParams {
                        muzzle_offset: weapon.muzzle_offset * (1.0 + 0.8 * i as f32),
                        ..weapon
                    },
                )
            })
            .collect(),
        BarrelVariant::SailShred => {
            let mut shred = barrel(first_id, owner_ship_id, ship, heading, weapon);
            shred.damage = (shred.damage as f32 * SAILSHRED_HULL).round() as u32;
            shred.sail_damage *= SAILSHRED_SAIL;
            vec![shred]
        }
    }
}

/// Barril incendiário na esteira: parado, dura `BARREL_LIFETIME_SECS`.
pub fn barrel(
    projectile_id: u32,
    owner_ship_id: u32,
    ship: (f32, f32),
    heading: f32,
    weapon: WeaponParams,
) -> Projectile {
    let astern = heading + std::f32::consts::PI;
    let mut barrel = Projectile::aimed(
        projectile_id,
        owner_ship_id,
        ship.0,
        ship.1,
        astern,
        WeaponParams {
            // Sai da popa com folga para não explodir no próprio rastro
            // de quem vem logo atrás… nem no dono (o dono nunca é alvo).
            muzzle_offset: weapon.muzzle_offset * 2.5,
            ..scaled(weapon, BARREL_DAMAGE)
        },
    );
    barrel.speed = 0.0;
    barrel.remaining_lifetime = BARREL_LIFETIME_SECS;
    barrel.kind = ProjectileKind::Barrel;
    barrel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapon() -> WeaponParams {
        WeaponParams {
            damage: 10,
            speed: 150.0,
            range: 500.0,
            muzzle_offset: 9.0,
        }
    }

    #[test]
    fn fan_spreads_around_the_bearing_and_keeps_total_damage() {
        let balls = fan_salvo(1, 7, (0.0, 0.0), (0.0, 0.0), 0.0, weapon());
        assert_eq!(balls.len(), FAN_BALLS as usize);
        let hull: u32 = balls.iter().map(|b| b.damage).sum();
        // 22 de dano bruto → 80% no casco, repartido sem sumir dano.
        let total = scaled(weapon(), FAN_DAMAGE).damage;
        let expected: u32 = (0..FAN_BALLS)
            .map(|i| {
                let per = total / FAN_BALLS
                    + if i == FAN_BALLS / 2 {
                        total % FAN_BALLS
                    } else {
                        0
                    };
                (per as f32 * 0.8).round() as u32
            })
            .sum();
        assert_eq!(hull, expected);
        let first = balls[0].heading.rem_euclid(std::f32::consts::TAU);
        let last = balls[FAN_BALLS as usize - 1].heading;
        assert!((last - FAN_SPREAD / 2.0).abs() < 1e-4);
        assert!((first - (std::f32::consts::TAU - FAN_SPREAD / 2.0)).abs() < 1e-4);
        assert!(balls.iter().all(|b| b.owner_ship_id == 7));
    }

    #[test]
    fn barrel_floats_astern_and_hits_harder() {
        let b = barrel(1, 7, (100.0, 0.0), 0.0, weapon());
        assert_eq!(b.kind, ProjectileKind::Barrel);
        assert_eq!(b.speed, 0.0);
        assert!(b.x < 100.0, "nasce na popa");
        assert!(b.damage > (weapon().damage as f32 * 0.8) as u32);
        let mut moved = b;
        moved.advance(1.0);
        assert_eq!((moved.x, moved.y), (b.x, b.y), "barril não anda");
    }

    #[test]
    fn cooldowns_gate_skills_and_ram_hits_each_target_once() {
        let mut combat = ActiveCombat::default();
        assert!(combat.try_fan());
        assert!(!combat.try_fan(), "leque em recarga");
        combat.tick(FAN_COOLDOWN_SECS);
        assert!(combat.try_fan());

        assert!(!combat.ram_hit(5), "sem arrancada não há choque");
        assert!(combat.try_ram(false));
        assert!(combat.ram_hit(5));
        assert!(!combat.ram_hit(5), "um choque por alvo");
        assert!(combat.ram_hit(6));
        combat.tick(RAM_SECS);
        assert!(!combat.is_ramming());
        assert!(!combat.try_ram(false), "abalroar em recarga");
    }

    #[test]
    fn gems_turn_skills_into_their_variants() {
        let none = SkillVariants::from_gems([]);
        assert_eq!(none, SkillVariants::default());
        let v = SkillVariants::from_gems([GemKind::Ruby, GemKind::Sapphire, GemKind::Amethyst]);
        assert_eq!(v.fan, FanVariant::Precision, "Safira vence o Rubi");
        assert!(v.long_ram);
        assert_eq!(
            SkillVariants::from_gems([GemKind::Diamond, GemKind::Topaz]).barrel,
            BarrelVariant::Triple
        );
        assert_eq!(
            SkillVariants::from_gems([GemKind::Emerald]).wire(),
            [FanVariant::DoubleFan as u8, 0, 0]
        );
    }

    #[test]
    fn each_variant_shoots_what_it_promises() {
        let w = weapon();
        let at = (0.0, 0.0);
        let still = (0.0, 0.0);
        let precision = fan_skill(FanVariant::Precision, 1, 7, at, still, 0.0, w);
        assert_eq!(precision.len(), 1);
        assert!(precision[0].damage > fan_salvo(1, 7, at, still, 0.0, w)[3].damage * 3);
        assert!(precision[0].speed > w.speed);
        let fire = fan_skill(FanVariant::Incendiary, 1, 7, at, still, 0.0, w);
        assert!(fire.iter().all(|b| b.kind == ProjectileKind::Fire));
        let double = fan_skill(FanVariant::DoubleFan, 1, 7, at, still, 0.0, w);
        assert_eq!(double.len() as u32, fan_skill_ids(FanVariant::DoubleFan));
        let ids: std::collections::HashSet<u32> = double.iter().map(|b| b.projectile_id).collect();
        assert_eq!(ids.len(), double.len(), "ids únicos");

        let triple = barrel_skill(BarrelVariant::Triple, 1, 7, (100.0, 0.0), 0.0, w);
        assert_eq!(triple.len(), 3);
        assert!(triple[0].x > triple[2].x, "enfileirados na esteira");
        let plain = barrel_skill(BarrelVariant::Barrel, 1, 7, at, 0.0, w)[0];
        let shred = barrel_skill(BarrelVariant::SailShred, 1, 7, at, 0.0, w)[0];
        assert!(shred.damage < plain.damage && shred.sail_damage > plain.sail_damage * 3.0);

        let mut combat = ActiveCombat::default();
        assert!(combat.try_ram(true));
        combat.tick(RAM_SECS + 0.1);
        assert!(combat.is_ramming(), "abalroar longo dura mais");
        combat.tick(LONG_RAM_COOLDOWN_SECS);
        assert!(combat.try_ram(true));
    }

    #[test]
    fn manual_fire_silences_the_auto_gunner_for_a_while() {
        let mut combat = ActiveCombat::default();
        assert!(!combat.auto_silenced());
        combat.mark_manual();
        assert!(combat.auto_silenced());
        combat.tick(MANUAL_HOLD_SECS);
        assert!(!combat.auto_silenced());
    }
}
