"""Ícones de item do Marvyr (atlas `assets/marvyr/ui/items.png`).

Chamado por `marvyr_art.py`. Mesmas convenções: pixel art determinística,
luz do noroeste (borda NO acesa, SE escura) e contorno escuro de 1 px.

Layout (o client monta o atlas com os mesmos números, `items_layout`):
  linha 0 (y=0):  8 peças de equipamento 24x24, passo 26
  linha 1 (y=26): 8 recursos 24x24, passo 26
  linha 2 (y=52): 3 molduras de raridade 32x32, passo 34
  linha 3 (y=86): 6 gemas + encaixe vazio 16x16, passo 18
  linha 4 (y=104): 4 frascos cheios + vidro vazio 24x24, passo 26
  linha 5 (y=130): 5 orbes de ofício 24x24, passo 26
"""

import math

from PIL import Image

OUTLINE = (24, 20, 26)

# Materiais: (aceso, médio, escuro)
WOOD = [(196, 146, 92), (158, 112, 70), (112, 76, 48)]
IRON = [(176, 184, 194), (122, 130, 142), (78, 84, 96)]
BRONZE = [(240, 186, 102), (198, 136, 60), (138, 88, 36)]
BLACK = [(96, 92, 104), (58, 56, 66), (34, 32, 40)]
FOG = [(214, 226, 230), (164, 186, 196), (116, 138, 152)]
ABYSS = [(176, 120, 214), (122, 72, 168), (76, 40, 112)]
CRYSTAL = [(200, 250, 255), (120, 214, 236), (60, 150, 190)]
CANVAS = [(246, 238, 216), (220, 206, 176), (176, 160, 128)]

EQUIPMENT = [
    ("Casco Reforçado", "hull", WOOD, IRON),
    ("Velas de Corrida", "sail", CANVAS, WOOD),
    ("Canhão de Bronze", "cannon", BRONZE, WOOD),
    ("Casco Negro", "hull", BLACK, BRONZE),
    ("Velas de Cerração", "sail", FOG, BLACK),
    ("Canhões Abissais", "cannon", ABYSS, BLACK),
    ("Casco de Cristal", "hull", CRYSTAL, IRON),
    ("Canhões de Cristal", "cannon", CRYSTAL, IRON),
]

RESOURCES = [
    "Madeira",
    "Minério",
    "Coral Negro",
    "Pérola Abissal",
    "Essência da Cerração",
    "Âmbar Abissal",
    "Cristal da Cerração",
    "Mapa do Tesouro",
]

GEMS = [
    (232, 60, 70),  # rubi
    (70, 120, 240),  # safira
    (60, 200, 110),  # esmeralda
    (250, 196, 60),  # topázio
    (170, 90, 230),  # ametista
    (236, 244, 250),  # diamante
]


def blank(size):
    return Image.new("RGBA", (size, size), (0, 0, 0, 0))


def px(img, x, y, color):
    if 0 <= x < img.width and 0 <= y < img.height:
        img.putpixel((x, y), tuple(color[:3]) + (255 if len(color) == 3 else color[3],))


def opaque(img, x, y):
    return 0 <= x < img.width and 0 <= y < img.height and img.getpixel((x, y))[3] > 0


def fill(img, inside, pal, grain=None):
    """Pinta onde `inside(x, y)` com o tom médio; `grain(x, y)` escolhe tom
    0/1/2 para textura."""
    for y in range(img.height):
        for x in range(img.width):
            if inside(x, y):
                tone = grain(x, y) if grain else 1
                px(img, x, y, pal[tone])


def light(img, pal):
    """Luz do noroeste: pixel com vazio ao NO acende, com vazio ao SE
    escurece."""
    src = img.copy()
    for y in range(img.height):
        for x in range(img.width):
            if not opaque(src, x, y):
                continue
            if not opaque(src, x - 1, y) or not opaque(src, x, y - 1):
                px(img, x, y, pal[0])
            elif not opaque(src, x + 1, y) or not opaque(src, x, y + 1):
                px(img, x, y, pal[2])


def outline(img):
    src = img.copy()
    for y in range(img.height):
        for x in range(img.width):
            if opaque(src, x, y):
                continue
            if any(opaque(src, x + dx, y + dy) for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
                px(img, x, y, OUTLINE)


def finish(img, pal):
    light(img, pal)
    outline(img)
    return img


# ── Equipamento ────────────────────────────────────────────────────────


def hull(main, trim):
    """Seção do casco vista de frente: tábuas em U, cinta e rebites."""
    img = blank(24)

    def inside(x, y):
        if y < 4 or y > 20:
            return False
        half = 9 if y < 13 else 9 - (y - 12) * 1.1
        return abs(x - 11.5) <= half

    fill(img, inside, main, lambda x, y: 2 if y % 4 == 3 else 1)
    finish(img, main)
    # Cinta de metal no meio, com rebites acesos.
    for x in range(3, 21):
        if opaque(img, x, 9) and img.getpixel((x, 9))[:3] != OUTLINE:
            px(img, x, 9, trim[1])
            px(img, x, 10, trim[2])
    for x in (5, 11, 17):
        px(img, x, 9, trim[0])
    # Borda de convés acesa.
    for x in range(3, 21):
        if img.getpixel((x, 4))[:3] != OUTLINE and opaque(img, x, 4):
            px(img, x, 4, main[0])
    return img


def sail(main, trim):
    """Vela enfunada num mastro, verga no alto e flâmula."""
    img = blank(24)
    for y in range(2, 22):
        px(img, 11, y, trim[1])
        px(img, 12, y, trim[2])
    for x in range(4, 20):
        px(img, x, 5, trim[1])

    def inside(x, y):
        if y < 6 or y > 18:
            return False
        bulge = 1.8 * math.sin((y - 6) / 12 * math.pi)
        return 4 - bulge <= x <= 19 + bulge and not (11 <= x <= 12)

    fill(img, inside, main, lambda x, y: 2 if (x + 2 * y) % 9 == 0 else 1)
    finish(img, main)
    # Flâmula vermelha no topo.
    for i, x in enumerate(range(13, 17)):
        px(img, x, 2 + i // 2, (200, 60, 50))
    return img


def cannon(main, trim):
    """Canhão em diagonal (boca a nordeste) sobre reparo de madeira."""
    img = blank(24)

    def barrel(x, y):
        # Eixo de (5,16) a (19,6); raio que afina para a boca.
        ax, ay, bx, by = 5, 16, 19, 6
        t = ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / ((bx - ax) ** 2 + (by - ay) ** 2)
        if not 0 <= t <= 1:
            return False
        cx, cy = ax + t * (bx - ax), ay + t * (by - ay)
        radius = 3.2 - 1.2 * t
        return math.hypot(x - cx, y - cy) <= radius

    fill(img, barrel, main)
    finish(img, main)
    # Boca escura.
    px(img, 19, 6, OUTLINE)
    px(img, 20, 6, OUTLINE)
    # Reparo e roda.
    for y in range(16, 21):
        for x in range(6, 16):
            if not opaque(img, x, y):
                px(img, x, y, trim[1] if y > 16 else trim[0])
    for dx in range(-2, 3):
        for dy in range(-2, 3):
            if dx * dx + dy * dy <= 5:
                px(img, 8 + dx, 20 + dy, trim[2] if dx * dx + dy * dy > 1 else trim[0])
    outline(img)
    return img


# ── Recursos ───────────────────────────────────────────────────────────


def logs():
    img = blank(24)
    for cy in (9, 15):
        fill(img, lambda x, y, cy=cy: 3 <= x <= 20 and abs(y - cy) <= 2, WOOD)
        for dx in range(-2, 3):
            for dy in range(-2, 3):
                if dx * dx + dy * dy <= 5:
                    ring = (224, 188, 130) if dx * dx + dy * dy > 1 else WOOD[2]
                    px(img, 20 + dx, cy + dy, ring)
    return finish(img, WOOD)


def ore():
    img = blank(24)
    pts = [(5, 14), (8, 7), (14, 5), (19, 9), (20, 16), (13, 20), (7, 19)]

    def inside(x, y):
        # Polígono convexo simples.
        for (x1, y1), (x2, y2) in zip(pts, pts[1:] + pts[:1]):
            if (x2 - x1) * (y - y1) - (y2 - y1) * (x - x1) < 0:
                return False
        return True

    fill(img, inside, IRON, lambda x, y: 2 if (x * 3 + y) % 7 == 0 else 1)
    finish(img, IRON)
    for x, y in [(10, 10), (15, 13), (11, 16)]:
        px(img, x, y, BRONZE[0])
        px(img, x + 1, y, BRONZE[1])
    return img


def coral():
    img = blank(24)
    pal = [(150, 60, 70), (100, 30, 44), (60, 16, 28)]
    branches = [((12, 21), (12, 6)), ((12, 14), (6, 7)), ((12, 12), (18, 5)), ((8, 10), (5, 4)), ((16, 9), (20, 11))]
    for (x1, y1), (x2, y2) in branches:
        steps = max(abs(x2 - x1), abs(y2 - y1))
        for i in range(steps + 1):
            x = round(x1 + (x2 - x1) * i / steps)
            y = round(y1 + (y2 - y1) * i / steps)
            px(img, x, y, pal[1])
            px(img, x + 1, y, pal[1])
    return finish(img, pal)


def pearl():
    img = blank(24)
    shell = [(214, 196, 176), (170, 150, 132), (120, 104, 94)]
    fill(img, lambda x, y: y >= 12 and math.hypot(x - 11.5, y - 12) <= 9, shell, lambda x, y: 2 if x % 3 == 0 else 1)
    finish(img, shell)
    white = [(250, 250, 255), (214, 220, 236), (160, 168, 196)]
    ball = blank(24)
    fill(ball, lambda x, y: math.hypot(x - 11.5, y - 11) <= 5, white)
    finish(ball, white)
    img.alpha_composite(ball)
    px(img, 9, 8, (255, 255, 255))
    return img


def essence():
    img = blank(24)
    glass = [(214, 236, 244), (160, 196, 214), (110, 140, 160)]
    fill(img, lambda x, y: (y >= 9 and math.hypot(x - 11.5, y - 14) <= 7) or (10 <= x <= 13 and 3 <= y <= 9), glass)
    finish(img, glass)
    # Névoa girando dentro.
    for i in range(24):
        a = i / 24 * 2 * math.pi * 1.5
        r = 1 + i / 5
        px(img, round(11.5 + math.cos(a) * r), round(14 + math.sin(a) * r * 0.8), (236, 248, 255))
    px(img, 11, 2, WOOD[1])
    px(img, 12, 2, WOOD[2])
    return img


def amber():
    img = blank(24)
    pal = [(255, 196, 80), (224, 140, 30), (160, 86, 14)]
    fill(img, lambda x, y: (y >= 10 and math.hypot(x - 11.5, y - 14) <= 7) or (y < 10 and abs(x - 11.5) <= (y - 3) * 0.8), pal)
    finish(img, pal)
    px(img, 12, 14, (70, 40, 20))  # inseto preso
    px(img, 13, 15, (70, 40, 20))
    px(img, 9, 11, (255, 236, 180))
    return img


def crystal():
    """Três lascas pontudas; a do meio, maior, na frente."""
    img = blank(24)
    for cx, h, w in [(7, 11, 2.5), (17, 9, 2.5), (12, 16, 3.5)]:
        top = 21 - h
        shard = blank(24)
        fill(
            shard,
            lambda x, y, cx=cx, top=top, w=w: top <= y <= 20
            and abs(x - cx) <= min(w, (y - top) * w / 3),
            CRYSTAL,
            lambda x, y, cx=cx: 0 if x < cx else 2 if x > cx else 1,
        )
        finish(shard, CRYSTAL)
        img.alpha_composite(shard)
    return img


def treasure_map():
    img = blank(24)
    paper = [(246, 228, 184), (226, 198, 142), (180, 146, 96)]
    fill(img, lambda x, y: 3 <= x <= 20 and 5 <= y <= 18, paper)
    finish(img, paper)
    for i in range(7):  # trilha tracejada
        if i % 2 == 0:
            px(img, 6 + i, 14 - i // 2, (140, 60, 40))
    for d in range(-2, 3):  # X vermelho
        px(img, 16 + d, 9 + d, (200, 40, 40))
        px(img, 16 + d, 9 - d, (200, 40, 40))
    return img


# ── Molduras de raridade ───────────────────────────────────────────────


def frame(rarity):
    """Moldura 32x32 para o ícone: madeira (Normal), prata com brilho azul
    (Mágico), ouro com brilho e pedras nos cantos (Raro)."""
    img = blank(32)
    pal = {
        0: WOOD,
        1: [(200, 220, 255), (120, 150, 220), (60, 80, 150)],
        2: [(255, 230, 140), (222, 170, 50), (150, 100, 20)],
    }[rarity]
    glow = {0: None, 1: (90, 140, 255), 2: (255, 200, 60)}[rarity]
    if glow:
        # Halo de 2 px fora da moldura, esmaecendo.
        for y in range(32):
            for x in range(32):
                d = min(x, y, 31 - x, 31 - y)
                if d < 2:
                    px(img, x, y, glow + (110 if d == 1 else 50,))
    for y in range(2, 30):
        for x in range(2, 30):
            d = min(x - 2, y - 2, 27 - (x - 2), 27 - (y - 2))
            if d <= 2:
                tone = 0 if (x < 16 and y < 16) or d == 0 and (x == 2 or y == 2) else 1
                if d == 2:
                    tone = 2
                px(img, x, y, pal[tone])
            elif d == 3:
                px(img, x, y, OUTLINE)
    # Fundo do ícone: papel escurecido.
    for y in range(6, 26):
        for x in range(6, 26):
            px(img, x, y, (58, 50, 44, 150))
    if rarity == 2:
        for cx, cy in [(3, 3), (27, 3), (3, 27), (27, 27)]:
            for dx in range(2):
                for dy in range(2):
                    px(img, cx + dx, cy + dy, (232, 60, 70) if dx + dy else (255, 170, 170))
    if rarity == 1:
        for cx, cy in [(4, 4), (27, 4), (4, 27), (27, 27)]:
            px(img, cx, cy, (230, 240, 255))
    return img


# ── Gemas ──────────────────────────────────────────────────────────────


def gem(color):
    """Gema lapidada 16x16: coroa facetada clara, pavilhão escuro, brilho."""
    img = blank(16)
    r, g, b = color
    pal = [
        (min(255, r + 60), min(255, g + 60), min(255, b + 60)),
        color,
        (r * 3 // 5, g * 3 // 5, b * 3 // 5),
    ]

    def inside(x, y):
        if y < 3 or y > 13:
            return False
        if y <= 6:
            return abs(x - 7.5) <= 3 + (y - 3)
        return abs(x - 7.5) <= 6 - (y - 6) * 0.85

    fill(img, inside, pal, lambda x, y: 0 if y <= 6 and x < 8 else 2 if y > 6 and x > 7 else 1)
    finish(img, pal)
    px(img, 5, 5, (255, 255, 255))
    px(img, 6, 4, (255, 255, 255))
    return img


def socket():
    """Encaixe vazio: aro de latão com o fundo escuro."""
    img = blank(16)
    for y in range(16):
        for x in range(16):
            d = math.hypot(x - 7.5, y - 7.5)
            if d <= 5.5:
                px(img, x, y, (30, 26, 32))
            elif d <= 7:
                px(img, x, y, (226, 176, 80) if x + y < 15 else (140, 96, 30))
    return img


# ── Frascos ────────────────────────────────────────────────────────────

# Líquido de cada frasco (ordem de `FlaskKind::ALL`): Estopa, Vento,
# Fúria, Breu.
FLASKS = [
    (214, 52, 60),  # estopa: vermelho de vida
    (150, 226, 240),  # vento: ar engarrafado
    (250, 150, 40),  # fúria: pólvora acesa
    (104, 60, 132),  # breu: piche violeta
]
GLASS = [(236, 244, 246), (190, 210, 216), (132, 150, 160)]


def flask(liquid):
    """Frasco 24x24: rolha de madeira, gargalo e bojo redondo. `liquid`
    None = vidro vazio (o HUD sobe o líquido por cima conforme as cargas)."""
    img = blank(24)

    def body(x, y):
        return math.hypot(x - 11.5, y - 15.5) <= 7.2

    def neck(x, y):
        return 9 <= x <= 14 and 4 <= y <= 9

    fill(img, lambda x, y: body(x, y) or neck(x, y), GLASS, lambda x, y: 0 if x < 10 and y < 14 else 1)
    if liquid:
        r, g, b = liquid
        pal = [(min(255, r + 50), min(255, g + 50), min(255, b + 50)), liquid, (r * 3 // 5, g * 3 // 5, b * 3 // 5)]
        fill(
            img,
            lambda x, y: math.hypot(x - 11.5, y - 15.5) <= 6.0 and y >= 11,
            pal,
            lambda x, y: 0 if y == 11 else 2 if x > 13 and y > 16 else 1,
        )
    finish(img, GLASS)
    # Rolha por cima do contorno do gargalo.
    for y in range(1, 4):
        for x in range(9, 15):
            px(img, x, y, WOOD[0] if x < 11 else WOOD[2] if x > 12 else WOOD[1])
    for x in range(8, 16):
        px(img, x, 0, OUTLINE)
    px(img, 8, 1, OUTLINE), px(img, 8, 2, OUTLINE), px(img, 8, 3, OUTLINE)
    px(img, 15, 1, OUTLINE), px(img, 15, 2, OUTLINE), px(img, 15, 3, OUTLINE)
    # Reflexo no vidro.
    px(img, 8, 12, (255, 255, 255))
    px(img, 7, 13, (255, 255, 255))
    return img


# ── Orbes de ofício ────────────────────────────────────────────────────

# (cor da esfera, marca de dentro) na ordem de `OrbKind::ALL`.
ORBS = [
    ((110, 170, 250), "dot"),  # transmutação: azul, um ponto (vira Mágico)
    ((236, 190, 60), "swirl"),  # caos: ouro, redemoinho
    ((150, 80, 210), "crown"),  # régio: roxo real, coroa
    ((236, 240, 250), "star"),  # exaltado: prata, estrela
    ((60, 180, 170), "compass"),  # cartógrafo: verde-mar, rosa dos ventos
]


def orb(color, mark):
    """Esfera 24x24 com brilho, halo e a marca do orbe no miolo."""
    img = blank(24)
    r, g, b = color
    pal = [(min(255, r + 70), min(255, g + 70), min(255, b + 70)), color, (r * 2 // 5, g * 2 // 5, b * 2 // 5)]

    def inside(x, y):
        return math.hypot(x - 11.5, y - 11.5) <= 8.6

    def grain(x, y):
        d = math.hypot(x - 9.0, y - 9.0)
        return 0 if d < 3.5 else 1 if d < 7.5 else 2

    fill(img, inside, pal, grain)
    finish(img, pal)
    ink = (255, 255, 255) if mark != "star" else (120, 130, 160)
    cx, cy = 12, 12
    if mark == "dot":
        for dx in range(-2, 2):
            for dy in range(-2, 2):
                if abs(dx + 0.5) + abs(dy + 0.5) <= 2.5:
                    px(img, cx + dx, cy + dy, (20, 50, 120))
    elif mark == "swirl":
        for t in range(0, 34):
            a = t * 0.38
            rad = 1.0 + t * 0.14
            px(img, round(cx + math.cos(a) * rad), round(cy + math.sin(a) * rad), (120, 70, 10))
    elif mark == "crown":
        for x in range(8, 16):
            px(img, x, 15, ink)
            px(img, x, 14, ink)
        for x, top in [(8, 10), (11, 9), (12, 9), (15, 10)]:
            for y in range(top, 14):
                px(img, x, y, ink)
    elif mark == "star":
        for d in range(-4, 5):
            px(img, cx + d, cy, ink)
            px(img, cx, cy + d, ink)
        for d in range(-2, 3):
            px(img, cx + d, cy + d, ink)
            px(img, cx + d, cy - d, ink)
    elif mark == "compass":
        for d in range(-5, 6):
            px(img, cx, cy + d, ink)
        for d in range(-3, 4):
            px(img, cx + d, cy, ink)
        px(img, cx, cy - 6, (255, 90, 80))
        px(img, cx, cy - 5, (255, 90, 80))
    px(img, 8, 7, (255, 255, 255))
    px(img, 7, 8, (255, 255, 255))
    return img


def items_sheet(dst):
    icons_eq = [{"hull": hull, "sail": sail, "cannon": cannon}[shape](main, trim) for _, shape, main, trim in EQUIPMENT]
    icons_res = [logs(), ore(), coral(), pearl(), essence(), amber(), crystal(), treasure_map()]
    out = Image.new("RGBA", (8 * 26, 130 + 24), (0, 0, 0, 0))
    for i, icon in enumerate(icons_eq):
        out.alpha_composite(icon, (i * 26, 0))
    for i, icon in enumerate(icons_res):
        out.alpha_composite(icon, (i * 26, 26))
    for i in range(3):
        out.alpha_composite(frame(i), (i * 34, 52))
    for i, color in enumerate(GEMS):
        out.alpha_composite(gem(color), (i * 18, 86))
    out.alpha_composite(socket(), (len(GEMS) * 18, 86))
    for i, liquid in enumerate(FLASKS + [None]):
        out.alpha_composite(flask(liquid), (i * 26, 104))
    for i, (color, mark) in enumerate(ORBS):
        out.alpha_composite(orb(color, mark), (i * 26, 130))
    out.save(dst)
    print(f"items: {out.size} ({len(icons_eq)} peças, {len(icons_res)} recursos, 3 molduras, {len(GEMS)} gemas + encaixe)")
