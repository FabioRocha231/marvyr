# Asset Registry

Toda asset externa usada no Marvyr vive aqui. Nada entra em
`assets/external/` ou `assets/marvyr/` sem estar registrado.

## Política de Licença (fail-closed)
- **CC0 / Public Domain:** permitido.
- **CC-BY / CC-BY-SA:** revisão EXPLÍCITA do maintainer antes de usar.
  Attribution required fica no `ATTRIBUTION.md`.
- **Licença custom:** revisão EXPLÍCITA; sem aprovação, proibido.
- **Sem licença clara / "free for use":** PROIBIDO.

## Como registrar uma asset

1. Coloque o arquivo em `assets/external/<nome>.png` (ou em `assets/marvyr/` se for uma adaptação nossa).
2. Adicione uma entrada na tabela abaixo com:
   - nome do arquivo
   - autor original
   - URL de origem
   - licença
   - data de obtenção (YYYY-MM-DD)
   - attribution required (true/false)
   - arquivos utilizados (paths)
   - alterações realizadas (ou "nenhuma")
3. Se `attribution required = true`, adicione também ao `docs/assets/ATTRIBUTION.md`.
4. Commit. Sem aprovação de um maintainer, nada mergeia em main.

## Registro

| Pack | Autor | URL | Licença | Attribution required | Arquivos utilizados | Modificações | Data de inclusão |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Scallywag - Ships | Pixel Carvel (comissionado); distribuído por Foozle | https://foozlecc.itch.io/scallywag-ships | CC0 1.0 | false | `assets/external/scallywag/ships/ships-tiles.png` | Renomeado de `Ships tiles.png` ao extrair o tilesheet oficial; conteúdo inalterado. | 2026-08-30 |
| Scallywag - Water and Islands | Pixel Carvel (comissionado); distribuído por Foozle | https://foozlecc.itch.io/scallywag-water-islands | CC0 1.0 | false | `assets/external/scallywag/water-islands/water-island-tiles.png` | Renomeado de `Water and Island tiles.png` ao extrair o tilesheet oficial; conteúdo inalterado. | 2026-08-30 |
| Scallywag - Fort | Pixel Carvel (comissionado); distribuído por Foozle | https://foozlecc.itch.io/scallywag-fort | CC0 1.0 | false | `assets/external/scallywag/fort/fort-tiles.png` | Renomeado de `Fort Tiles.png` ao extrair o tilesheet oficial; conteúdo inalterado. | 2026-08-30 |
| Battle at Sea (sfx) | Thimras | https://opengameart.org/content/battle-at-sea | CC0 1.0 | false | `assets/external/oga-battle-at-sea/cannon_fire_1.ogg`, `assets/external/oga-battle-at-sea/cannon_hit_ship_short.ogg`, `assets/external/oga-battle-at-sea/cannon_miss_1.ogg`, `assets/external/oga-battle-at-sea/ship_destroyed_short.ogg` | nenhuma | 2026-09-23 |
| A Sailor's Chant (música) | Thimras | https://opengameart.org/content/a-sailors-chant | CC0 1.0 | false | `assets/external/oga-sailors-chant/oga_jam_menu_music_loopable_0.ogg` | nenhuma | 2026-09-23 |
| Beach Ocean Waves | jasinski (freesound #18363); enviado por qubodup | https://opengameart.org/content/beach-ocean-waves | CC0 1.0 | false | `assets/external/oga-beach-ocean-waves/waves_loop.ogg` | `wave_01`..`wave_04` (.flac) concatenados, mono 22.05 kHz com fade de 0.3 s nas pontas para o loop, reencodado em Ogg Vorbis estéreo 44.1 kHz (ffmpeg) | 2026-09-23 |
| Short Wind Sound | remaxim | https://opengameart.org/content/short-wind-sound | CC0 1.0 | false | `assets/external/oga-short-wind/wind_loop.ogg` | Convertido para mono 22.05 kHz com fade de 0.5 s nas pontas para o loop, reencodado em Ogg Vorbis estéreo 44.1 kHz (ffmpeg) | 2026-09-23 |
| Kenney RPG Audio | Kenney | https://kenney.nl/assets/rpg-audio | CC0 1.0 | false | `assets/external/kenney-rpg-audio/creak1.ogg`, `assets/external/kenney-rpg-audio/handleCoins.ogg` | nenhuma | 2026-09-23 |
| Kenney Impact Sounds | Kenney | https://kenney.nl/assets/impact-sounds | CC0 1.0 | false | `assets/external/kenney-impact-sounds/impactBell_heavy_000.ogg` | nenhuma | 2026-09-23 |
| Kenney Interface Sounds | Kenney | https://kenney.nl/assets/interface-sounds | CC0 1.0 | false | `assets/external/kenney-interface-sounds/click_002.ogg` | nenhuma | 2026-09-23 |
| Alfa Slab One (fonte) | JM Solé | https://fonts.google.com/specimen/Alfa+Slab+One | SIL OFL 1.1 — **aguarda revisão do maintainer** (política: licença fora de CC0) | false (OFL exige manter o `OFL-AlfaSlabOne.txt` junto) | `assets/external/fonts/AlfaSlabOne-Regular.ttf`, `assets/external/fonts/OFL-AlfaSlabOne.txt` | nenhuma (embutida no binário do client) | 2026-09-24 |
| Zilla Slab (fonte) | Typotheque para a Mozilla Foundation | https://fonts.google.com/specimen/Zilla+Slab | SIL OFL 1.1 — **aguarda revisão do maintainer** (política: licença fora de CC0) | false (OFL exige manter o `OFL-ZillaSlab.txt` junto) | `assets/external/fonts/ZillaSlab-Regular.ttf`, `assets/external/fonts/ZillaSlab-SemiBold.ttf`, `assets/external/fonts/ZillaSlab-Bold.ttf`, `assets/external/fonts/OFL-ZillaSlab.txt` | nenhuma (embutida no binário do client) | 2026-09-24 |

## Assets geradas pelo próprio Marvyr

Sprites geradas pelo time do Marvyr para preencher HUD, combate e mar. O conteúdo é nosso, dedicado ao domínio público (CC0 1.0); sem attribution externa, então `ATTRIBUTION.md` permanece vazio para esta seção.

| Pack | Autor | URL | Licença | Attribution required | Arquivos utilizados | Modificações | Data de inclusão |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Marvyr Porto e bilhetes (upgrade pixel art) | Equipe Marvyr | (gerado por `tools/art/marvyr_art.py`) | CC0 1.0 | false | `assets/marvyr/world/port-buildings.png`, `assets/marvyr/ui/ticket.png` | nenhuma | 2026-09-25 |
| Marvyr ícones de item, molduras de raridade, gemas e frascos | Equipe Marvyr | (gerado por `tools/art/marvyr_icons.py`) | CC0 1.0 | false | `assets/marvyr/ui/items.png` | nenhuma | 2026-09-26 |
| Marvyr aura de poder (chamas em volta do casco) | Equipe Marvyr | (gerado por `tools/art/marvyr_art.py`) | CC0 1.0 | false | `assets/marvyr/ships/aura.png` | nenhuma | 2026-09-26 |
| Scallywag - Ships (cascos e velas Marvyr) | Pixel Carvel (comissionado); distribuído por Foozle + Equipe Marvyr | https://foozlecc.itch.io/scallywag-ships | CC0 1.0 | false | `assets/marvyr/ships/ships.png` | Derivado de `assets/external/scallywag/ships/ships-tiles.png`: cascos redesenhados nos mesmos recortes e velas/vergas novas numa faixa acrescentada (`tools/art/marvyr_art.py`); onda de proa, fumaça, fogo e cesto são do pack | 2026-09-25 |
| Scallywag - Fort (pedra clara) | Pixel Carvel (comissionado); distribuído por Foozle | https://foozlecc.itch.io/scallywag-fort | CC0 1.0 | false | `assets/marvyr/world/fort-stone.png` | Derivado de `assets/external/scallywag/fort/fort-tiles.png`: pixels de pedra roxa recoloridos para pedra clara quente (mesmo valor, `tools/art/marvyr_art.py`) | 2026-09-25 |
