"""Gerador da arte própria do Marvyr (pixel art por cima do pack Scallywag).

Uso: python3 tools/art/marvyr_art.py   (precisa de Pillow)

Tudo é determinístico: rodar de novo produz os mesmos PNGs em
assets/marvyr/. Convenções da arte: 1 px = 1 m no mundo, vista de cima,
luz fixa do noroeste (a mesma do shader do mar), contorno escuro de 1 px e
paleta curta por material.
"""

import colorsys
import os
import random

from PIL import Image

ROOT = os.path.join(os.path.dirname(__file__), "..", "..", "assets")
OUT = os.path.join(ROOT, "marvyr")

SHADOW = (20, 28, 40, 90)

# Paletas: (aceso, médio, escuro, face na sombra, contorno)
RED_TILE = [(214, 110, 70), (184, 80, 56), (148, 58, 46), (104, 40, 38), (60, 26, 30)]
SLATE = [(132, 146, 164), (104, 116, 138), (78, 88, 110), (56, 62, 82), (34, 38, 52)]
PLANK = [(184, 138, 88), (156, 110, 68), (124, 84, 52), (94, 62, 42), (58, 40, 30)]
THATCH = [(226, 190, 110), (200, 160, 84), (164, 124, 62), (126, 92, 48), (80, 58, 34)]
STONE = [(206, 198, 180), (170, 162, 146), (134, 127, 116), (100, 95, 90), (58, 55, 56)]


def put(img, x, y, color):
    if 0 <= x < img.width and 0 <= y < img.height:
        img.putpixel((x, y), color if len(color) == 4 else color + (255,))


def shade(img, x, y):
    """Escurece o pixel já pintado (sombra por cima do telhado)."""
    if 0 <= x < img.width and 0 <= y < img.height:
        r, g, b, a = img.getpixel((x, y))
        if a == 255:
            img.putpixel((x, y), (r * 2 // 3, g * 2 // 3, b * 2 // 3, 255))
        else:
            img.putpixel((x, y), SHADOW)


def line_tone(tone):
    """Fiada: mais escura nas águas acesas, mais clara na água da sombra
    (senão a água sul vira um bloco chapado)."""
    return tone + 1 if tone < 3 else 2


def hip_roof(w, h, pal, pattern="tile", chimney=False, seed=0):
    """Telhado de quatro águas visto de cima. A água norte e a oeste pegam
    luz, a sul fica na sombra; cumeeira no eixo maior. Sombra projetada de
    3 px para sudeste já vem no sprite."""
    rng = random.Random(seed)
    pad = 4
    img = Image.new("RGBA", (w + pad, h + pad), (0, 0, 0, 0))
    for y in range(h):
        for x in range(w):
            put(img, x + 3, y + 3, SHADOW)
    ridge_horizontal = w >= h
    for y in range(h):
        for x in range(w):
            left, right, top, bottom = x, w - 1 - x, y, h - 1 - y
            if ridge_horizontal:
                # Águas norte/sul dominam; oeste/leste são os triângulos.
                face = "n" if top <= bottom else "s"
                if min(left, right) < min(top, bottom):
                    face = "w" if left < right else "e"
            else:
                face = "w" if left <= right else "e"
                if min(top, bottom) < min(left, right):
                    face = "n" if top < bottom else "s"
            tone = {"n": 0, "w": 1, "e": 2, "s": 3}[face]
            color = pal[tone]
            # Fiadas de telha/tábua: linha mais escura a cada 3 px.
            along = top if face in "ns" else left if face == "w" else right
            if face == "s":
                along = bottom
            if pattern == "tile" and along % 3 == 2:
                color = pal[line_tone(tone)]
            if pattern == "plank":
                run = x if face in "ns" else y
                if run % 5 == 0 or (along % 4 == 3 and rng.random() < 0.5):
                    color = pal[line_tone(tone)]
            if pattern == "thatch" and rng.random() < 0.18:
                color = pal[line_tone(tone)]
            put(img, x, y, color)
    # Cumeeira acesa e contorno.
    for y in range(h):
        for x in range(w):
            edge = x in (0, w - 1) or y in (0, h - 1)
            if edge:
                put(img, x, y, pal[4])
    if ridge_horizontal:
        ry = h // 2 - 1
        for x in range(h // 2, w - h // 2):
            put(img, x, ry, pal[0])
    else:
        rx = w // 2 - 1
        for y in range(w // 2, h - w // 2):
            put(img, rx, y, pal[0])
    if chimney:
        cx, cy = w * 2 // 3, h // 4
        for y in range(cy, cy + 4):
            for x in range(cx, cx + 4):
                shade(img, x + 2, y + 2)
        for y in range(cy, cy + 4):
            for x in range(cx, cx + 4):
                c = STONE[1] if (x + y) % 2 else STONE[2]
                if x == cx or y == cy:
                    c = STONE[0]
                put(img, x, y, c)
        put(img, cx + 1, cy + 1, (30, 30, 34))
        put(img, cx + 2, cy + 1, (30, 30, 34))
        put(img, cx + 1, cy + 2, (30, 30, 34))
        put(img, cx + 2, cy + 2, (30, 30, 34))
    return img


def stall(w=18, h=12):
    """Barraca de mercado: toldo listrado + caixotes na frente."""
    img = Image.new("RGBA", (w + 4, h + 7), (0, 0, 0, 0))
    for y in range(h):
        for x in range(w):
            put(img, x + 2, y + 2, SHADOW)
    cream, red = (236, 224, 196), (196, 70, 58)
    cream_d, red_d = (200, 186, 158), (150, 50, 46)
    for y in range(h):
        for x in range(w):
            shade = y >= h // 2
            stripe = (x // 3) % 2 == 0
            c = (red_d if shade else red) if stripe else (cream_d if shade else cream)
            if x in (0, w - 1) or y in (0, h - 1):
                c = (70, 36, 34)
            put(img, x, y, c)
    # Franja do toldo.
    for x in range(1, w - 1, 2):
        put(img, x, h, (70, 36, 34))
    # Caixotes com mercadoria.
    for i, fruit in enumerate([(236, 164, 52), (120, 170, 70), (200, 60, 50)]):
        bx = 1 + i * 6
        for y in range(h + 2, h + 6):
            for x in range(bx, bx + 5):
                c = PLANK[1] if y > h + 2 else fruit
                if x in (bx, bx + 4) or y == h + 5:
                    c = PLANK[4]
                put(img, x, y, c)
    return img


def recolor_stone(src, dst):
    """As peças de pedra do pack são roxas; aqui viram pedra clara quente
    mantendo o valor de cada pixel (o desenho não muda)."""
    img = Image.open(src).convert("RGBA")
    px = img.load()
    for y in range(img.height):
        for x in range(img.width):
            r, g, b, a = px[x, y]
            if a == 0:
                continue
            h, s, v = colorsys.rgb_to_hsv(r / 255, g / 255, b / 255)
            if 0.60 <= h <= 0.80 and s > 0.08:
                nh, ns = 0.10, s * 0.30
                nv = min(1.0, v * 1.18 + 0.04)
                nr, ng, nb = colorsys.hsv_to_rgb(nh, ns, nv)
                px[x, y] = (int(nr * 255), int(ng * 255), int(nb * 255), a)
    black_flag_row(img)
    img.save(dst)


# Bandeiras do fort: 8x9 em (392 + frame*16, 99 + cor*16); a cor 5 é a
# branca. A Bandeira Negra (cor 6) sai dela: pano escuro e caveira em dois
# pixels claros.
FLAG_X0, FLAG_Y0, FLAG_STEP = 392, 99, 16
BLACK_FLAG = 6


def black_flag_row(img):
    px = img.load()
    tones = {115: (70, 64, 72), 200: (20, 18, 24), 225: (34, 30, 38)}
    for frame in range(3):
        x0 = FLAG_X0 + frame * FLAG_STEP
        src_y = FLAG_Y0 + 5 * FLAG_STEP
        dst_y = FLAG_Y0 + BLACK_FLAG * FLAG_STEP
        cloth = []
        for y in range(9):
            for x in range(8):
                r, g, b, a = px[x0 + x, src_y + y]
                if a == 0:
                    continue
                nearest = min(tones, key=lambda t: abs(t - r))
                px[x0 + x, dst_y + y] = tones[nearest] + (a,)
                if nearest != 115:
                    cloth.append((x, y))
        # Caveira: os dois pixels de pano mais centrais da linha do meio.
        mid = sorted(cloth, key=lambda p: (abs(p[1] - 4), abs(p[0] - 3.5)))[:2]
        for x, y in mid:
            px[x0 + x, dst_y + y] = (236, 230, 214, 255)


def sheet(parts, path):
    """Empilha as peças numa linha só (com 2 px de folga) e devolve os
    recortes, para `assets.rs` montar o atlas."""
    width = sum(p.width + 2 for p in parts)
    height = max(p.height for p in parts)
    out = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    x = 0
    rects = []
    for p in parts:
        out.paste(p, (x, 0), p)
        rects.append((x, 0, p.width, p.height))
        x += p.width + 2
    out.save(path)
    return rects


def ticket_frame(scale=2):
    """Moldura 9-slice dos bilhetes do HUD: papel de trapo com bisel (luz
    no alto/esquerda, sombra embaixo/direita), fio de tinta com canto
    chanfrado e rebite de latão em cada canto. Borda de 4 px de arte;
    `scale` amplia sem suavizar (cada pixel de arte vira scale x scale)."""
    size, border = 16, 4
    ink = (22, 32, 43)
    paper = (233, 220, 192)
    paper_shade = (216, 199, 162)
    light = (247, 240, 222)
    dark = (196, 176, 136)
    brass, brass_dark, brass_light = (217, 164, 65), (122, 78, 14), (246, 214, 128)
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    last = size - 1
    for y in range(size):
        for x in range(size):
            c = paper
            inner = min(x, y, last - x, last - y)
            if inner == 2:
                c = paper_shade
            if inner == 1:
                c = light if (x == 1 or y == 1) and x < last - 1 and y < last - 1 else dark
            if inner == 0:
                c = ink
            put(img, x, y, c)
    # Canto chanfrado: o pixel do canto some.
    for x, y in [(0, 0), (last, 0), (0, last), (last, last)]:
        img.putpixel((x, y), (0, 0, 0, 0))
    for x, y in [(1, 1), (last - 1, 1), (1, last - 1), (last - 1, last - 1)]:
        put(img, x, y, ink)
    # Rebites de latão (2x2) nos cantos, com brilho no noroeste.
    for cx, cy in [(2, 2), (last - 3, 2), (2, last - 3), (last - 3, last - 3)]:
        put(img, cx, cy, brass_light)
        put(img, cx + 1, cy, brass)
        put(img, cx, cy + 1, brass)
        put(img, cx + 1, cy + 1, brass_dark)
    return img.resize((size * scale, size * scale), Image.NEAREST), border * scale


# ── Navios ─────────────────────────────────────────────────────────────
# O sheet dos navios é o do Scallywag com cascos, velas e vergas nossos: os
# cascos ocupam os MESMOS recortes (tamanho de casco não muda), velas e
# vergas maiores moram numa faixa nova embaixo. Onda de proa, fumaça, fogo
# e cesto continuam os do pack. Proa para BAIXO na imagem (como no pack).

DECK = [(196, 150, 98), (172, 128, 80), (146, 104, 64), (112, 78, 48)]
HULL_OUTLINE = (40, 26, 22)
# Faixa pintada do costado por cor de casco (clara, escura) e a amurada.
HULL_BANDS = [
    ((110, 72, 46), (78, 50, 34), (150, 110, 70)),  # 0 marrom
    ((188, 150, 100), (150, 114, 72), (220, 190, 140)),  # 1 madeira clara
    ((176, 58, 48), (124, 38, 36), (214, 170, 120)),  # 2 vermelho
    ((52, 74, 120), (34, 48, 84), (200, 196, 180)),  # 3 azul-marinho
    ((214, 120, 44), (160, 80, 30), (238, 196, 88)),  # 4 laranja, friso dourado
]
SAIL_CLOTH = [
    ((244, 242, 234), (206, 202, 190)),  # branco
    ((238, 222, 184), (200, 180, 140)),  # creme
    ((106, 168, 92), (72, 124, 64)),  # verde
    ((236, 196, 76), (192, 150, 46)),  # amarelo
    ((86, 132, 196), (58, 94, 150)),  # azul
    ((202, 64, 54), (146, 40, 38)),  # vermelho
]
SPAR = [(150, 104, 62), (104, 70, 42), (60, 40, 28)]
CANNON = [(118, 124, 134), (44, 46, 52)]

# Recortes das peças novas (espelhados em `ship_parts_layout`).
# Pano quadrado passa da amurada: vela mais larga que o casco.
SAIL_SIZES = [(36, 11), (52, 14), (58, 17)]
SAIL_Y0, SAIL_ROW, SAIL_STEP = 680, 20, 120
YARD_SIZES = [(36, 12), (52, 12), (58, 15)]
YARD_Y0, YARD_ROW, YARD_STEP = 745, 17, 60
SHEET_H = 800


def hull_profile(w, h, t):
    """Meia-largura do casco na altura t (0 = popa, 1 = proa)."""
    if t < 0.10:
        frac = 0.80 + t * 2.0
    elif t < 0.58:
        frac = 1.0
    else:
        frac = max(0.0, ((1.0 - t) / 0.42)) ** 0.62
    return (w / 2.0) * frac


def draw_hull(w, h, color, style, damaged, seed):
    rng = random.Random(seed)
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    band, band_dark, rail = HULL_BANDS[color]
    sprit = 7 if h >= 80 else 5  # gurupés na proa
    body = h - sprit
    cx = (w - 1) / 2.0
    half = [hull_profile(w - 2, body, y / (body - 1)) for y in range(body)]
    for y in range(body):
        hw = half[y]
        for x in range(w):
            dx = abs(x - cx)
            if dx > hw + 0.5:
                continue
            edge = hw + 0.5 - dx
            if edge <= 1.0:
                c = HULL_OUTLINE
            elif edge <= 4.0:
                c = band if edge > 2.0 else band_dark
            elif edge <= 5.0:
                c = rail
            else:
                # Convés: tábuas no sentido do comprimento, emendas soltas.
                plank = int(x - cx + 40) // 3
                c = DECK[1] if plank % 2 else DECK[0]
                if (int(x - cx + 40)) % 3 == 0:
                    c = DECK[2]
                if (y + plank * 7) % 17 == 0:
                    c = DECK[3]
            put(img, x, y, c)
    # Proa: fecha a ponta com contorno; gurupés continua.
    for y in range(body - 1, h):
        for x in (int(cx), int(cx + 0.5)):
            put(img, x, y, SPAR[1] if y < h - 1 else SPAR[2])
    # Tombadilho (popa): degrau escuro + janelas de popa na borda.
    step_y = max(6, int(body * 0.16))
    for x in range(w):
        if abs(x - cx) < half[step_y] - 3.5:
            put(img, x, step_y, DECK[3])
    for x in range(w):
        if abs(x - cx) < half[0] - 4 and x % 3 != 0:
            put(img, x, 1, (238, 206, 110))
    # Leme/timão no tombadilho.
    put(img, int(cx), step_y // 2, SPAR[2])
    put(img, int(cx + 0.5), step_y // 2, SPAR[2])
    # Estilo por tipo.
    if style == "cargo":
        # Escotilha de carga com grade.
        top, bottom = int(body * 0.40), int(body * 0.58)
        left, right = int(cx - w * 0.2), int(cx + w * 0.2 + 0.5)
        for y in range(top, bottom + 1):
            for x in range(left, right + 1):
                c = DECK[3] if (x - left) % 3 == 0 or (y - top) % 3 == 0 else (46, 32, 26)
                if x in (left, right) or y in (top, bottom):
                    c = SPAR[2]
                put(img, x, y, c)
        # Caixotes no convés.
        for bx, by in [(int(cx - w * 0.25), int(body * 0.68)), (int(cx + 2), int(body * 0.24))]:
            for y in range(by, by + 4):
                for x in range(bx, bx + 4):
                    put(img, x, y, SPAR[0] if (x + y) % 3 else SPAR[1])
    if style in ("war", "raider"):
        guns = 4 if style == "war" else 2
        first, last = body * 0.26, body * 0.62
        for g in range(guns):
            y = int(first + (last - first) * (g / max(1, guns - 1)))
            hw = half[y]
            for side in (-1, 1):
                # Carreta no convés e cano saindo 1 px pelo costado.
                base = cx + side * (hw - 5)
                for k in range(7):
                    x = int(round(base + side * k))
                    top = CANNON[0] if k > 1 else SPAR[0]
                    put(img, x, y, top)
                    put(img, x, y + 1, CANNON[1] if k > 1 else SPAR[2])
    if style == "war":
        # Grade central do convés.
        top, bottom = int(body * 0.44), int(body * 0.54)
        for y in range(top, bottom + 1):
            for x in range(int(cx - 4), int(cx + 5)):
                put(img, x, y, (46, 32, 26) if (x + y) % 2 else DECK[3])
    if damaged:
        for _ in range(3 + w // 12):
            y = rng.randrange(step_y + 2, body - 6)
            x = int(cx + rng.uniform(-half[y] + 3, half[y] - 3))
            for dy in range(-1, 2):
                for dx in range(-1, 2):
                    if abs(dx) + abs(dy) < 2 or rng.random() < 0.4:
                        put(img, x + dx, y + dy, (22, 16, 14))
            put(img, x - 2, y - 1, DECK[0])
            put(img, x + 2, y + 1, DECK[0])
        for _ in range(w // 3):
            y = rng.randrange(2, body - 2)
            x = int(cx + rng.uniform(-half[y] + 1, half[y] - 1))
            if img.getpixel((x, y))[3]:
                put(img, x, y, (52, 44, 40))
        # Amurada quebrada: trechos sem faixa.
        for _ in range(2):
            y0 = rng.randrange(step_y, body - 10)
            side = rng.choice((-1, 1))
            for y in range(y0, y0 + 4):
                x = int(round(cx + side * (half[y] - 1.5)))
                put(img, x, y, DECK[3])
    return img


def draw_sail(w, h, color, full):
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    light, dark = SAIL_CLOTH[color]
    c = (w - 1) / 2.0
    if not full:
        # Vela recolhida: rolo fino amarrado na verga.
        y0 = h // 2 - 1
        for x in range(1, w - 1):
            for y in (y0, y0 + 1, y0 + 2):
                col = light if y == y0 else dark
                if y == y0 + 2:
                    col = HULL_OUTLINE
                put(img, x, y, col)
            if x % 6 == 3:
                put(img, x, y0, SPAR[1])
                put(img, x, y0 + 1, SPAR[1])
        return img
    # Vela cheia vista de cima: pano bojudo para a proa (para baixo).
    for x in range(w):
        u = (x - c) / (w / 2.0)
        depth = max(0.0, 1.0 - u * u) ** 0.5
        bottom = 1 + int(round((h - 2) * depth))
        for y in range(0, bottom + 1):
            col = light
            if y > bottom * 0.55 or abs(u) > 0.72:
                col = dark
            if x % 7 == 3 and y > 1:
                col = dark  # costura
            if y == bottom or x in (0, w - 1):
                col = HULL_OUTLINE
            put(img, x, y, col)
    for x in range(w):
        put(img, x, 0, HULL_OUTLINE)
    return img


def draw_yard(w, h, color):
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    y0 = h // 2 - 1
    for x in range(w):
        put(img, x, y0, SPAR[0])
        put(img, x, y0 + 1, SPAR[1])
    trim = SAIL_CLOTH[color][1]
    for x in (0, 1, w - 2, w - 1):
        put(img, x, y0, trim)
        put(img, x, y0 + 1, trim)
    # Mastro visto de cima: círculo de 4 px com topo aceso.
    cx = w // 2 - 2
    for y in range(y0 - 1, y0 + 3):
        for x in range(cx, cx + 4):
            corner = (x in (cx, cx + 3)) and (y in (y0 - 1, y0 + 2))
            if not corner:
                put(img, x, y, SPAR[2] if y == y0 + 2 or x == cx + 3 else SPAR[0])
    return img


def ship_sheet(dst):
    src = Image.open(os.path.join(ROOT, "external/scallywag/ships/ships-tiles.png")).convert("RGBA")
    out = Image.new("RGBA", (src.width, SHEET_H), (0, 0, 0, 0))
    out.paste(src, (0, 0))
    styles = ["raider", "cargo", "war"]  # pequeno, médio, grande
    for size, (x0, w, h) in enumerate([(1, 30, 64), (162, 44, 80), (401, 46, 128)]):
        step = 32 if w == 30 else 48
        for color in range(5):
            for damaged in (0, 1):
                x, y = x0 + color * step, damaged * h
                out.paste((0, 0, 0, 0), (x, y, x + w, y + h))
                hull = draw_hull(w, h, color, styles[size], bool(damaged), seed=size * 10 + color)
                out.paste(hull, (x, y), hull)
    for size, (w, h) in enumerate(SAIL_SIZES):
        for color in range(6):
            y = SAIL_Y0 + size * SAIL_ROW
            for full in (0, 1):
                x = color * SAIL_STEP + full * (SAIL_STEP // 2)
                sail = draw_sail(w, h, color, bool(full))
                out.paste(sail, (x, y), sail)
    for size, (w, h) in enumerate(YARD_SIZES):
        for color in range(6):
            yard = draw_yard(w, h, color)
            out.paste(yard, (color * YARD_STEP, YARD_Y0 + size * YARD_ROW), yard)
    out.save(dst)


# ── Aura de poder ──────────────────────────────────────────────────────
# Anel de chamas em volta do casco, visto de cima: brilho colado ao costado
# e línguas de fogo que variam por quadro. Uma linha por (cor, tamanho de
# casco), 4 quadros; o client escolhe a linha e anima.
AURA_HULLS = [(30, 64), (44, 80), (46, 128)]  # os mesmos de hull_px (client)
AURA_PAD = 28
AURA_FRAMES = 4
AURA_COLORS = [
    # (núcleo, meio, borda): dourado (poder) e rubro-negro (Bandeira Negra)
    [(255, 250, 214), (255, 196, 40), (196, 84, 8)],
    [(255, 120, 90), (176, 18, 32), (30, 6, 14)],
]
BAYER4 = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]


def aura_frame(hw, hh, frame, pal):
    """Brilho colado ao costado + línguas de fogo em lágrima que nascem no
    casco e se inclinam para a popa (topo da imagem: a proa do sprite fica
    embaixo), mais longas atrás e ondulando por quadro."""
    import math

    w, h = hw + 2 * AURA_PAD, hh + 2 * AURA_PAD
    heat = [[0.0] * w for _ in range(h)]
    cx, cy = (w - 1) / 2, (h - 1) / 2
    rx, ry = hw / 2, hh / 2

    def burn(x, y, value):
        xi, yi = int(round(x)), int(round(y))
        if 0 <= xi < w and 0 <= yi < h:
            heat[yi][xi] = max(heat[yi][xi], value)

    # Brilho junto ao casco.
    for y in range(h):
        for x in range(w):
            e = math.hypot((x - cx) / rx, (y - cy) / ry)
            if 0.96 <= e <= 1.26:
                burn(x, y, 0.66 - (e - 0.96) / 0.3 * 0.45)
    tongues = 14
    size = min(hh / 64, 1.3)
    for i in range(tongues):
        th = 2 * math.pi * i / tongues + frame * 0.11 + ((i * 37) % 7) * 0.05
        bx, by = cx + math.cos(th) * rx, cy + math.sin(th) * ry
        nx, ny = math.cos(th) / rx, math.sin(th) / ry
        norm = math.hypot(nx, ny)
        nx, ny = nx / norm, ny / norm
        stern = max(0.0, -math.sin(th))
        # Tudo se inclina para a popa; atrás, quase reto para trás.
        dx, dy = nx * (1 - 0.6 * stern), ny * (1 - 0.6 * stern) - 0.9
        d = math.hypot(dx, dy)
        dx, dy = dx / d, dy / d
        flicker = 0.65 + 0.35 * (((i * 7919 + frame * 104729) % 97) / 96)
        length = (7 + 26 * stern) * size * flicker
        width = 2.2 + 2.0 * stern
        steps = max(2, int(length * 2))
        for k in range(steps + 1):
            t = k / steps
            wiggle = math.sin(t * math.pi * 1.6 + frame * 1.57 + i) * 1.4 * t
            px_, py_ = bx + dx * length * t - dy * wiggle, by + dy * length * t + dx * wiggle
            radius = width * (1 - t) ** 0.8
            r = int(math.ceil(radius))
            for oy in range(-r, r + 1):
                for ox in range(-r, r + 1):
                    if ox * ox + oy * oy <= radius * radius:
                        burn(px_ + ox, py_ + oy, 1.0 - t * 0.85)
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    for y in range(h):
        for x in range(w):
            v = heat[y][x]
            if v <= 0:
                continue
            v += (BAYER4[y % 4][x % 4] / 16 - 0.5) * 0.2
            if v > 0.7:
                img.putpixel((x, y), pal[0] + (255,))
            elif v > 0.42:
                img.putpixel((x, y), pal[1] + (245,))
            elif v > 0.14:
                img.putpixel((x, y), pal[2] + (230,))
    return img


def aura_sheet(dst):
    """Linhas: cor 0 (três tamanhos), depois cor 1; colunas: quadros."""
    col_w = max(hw for hw, _ in AURA_HULLS) + 2 * AURA_PAD + 2
    rows = [(pal, hw, hh) for pal in AURA_COLORS for hw, hh in AURA_HULLS]
    height = sum(hh + 2 * AURA_PAD + 2 for _, _, hh in rows)
    out = Image.new("RGBA", (col_w * AURA_FRAMES, height), (0, 0, 0, 0))
    y = 0
    for pal, hw, hh in rows:
        for f in range(AURA_FRAMES):
            out.alpha_composite(aura_frame(hw, hh, f, pal), (f * col_w, y))
        print(f"aura {hw}x{hh}: y={y} passo={col_w}")
        y += hh + 2 * AURA_PAD + 2
    out.save(dst)
    print(f"aura sheet: {out.size}")


def main():
    os.makedirs(os.path.join(OUT, "world"), exist_ok=True)
    recolor_stone(
        os.path.join(ROOT, "external/scallywag/fort/fort-tiles.png"),
        os.path.join(OUT, "world/fort-stone.png"),
    )
    parts = [
        hip_roof(40, 26, PLANK, "plank", seed=1),  # WAREHOUSE
        hip_roof(22, 18, RED_TILE, "tile", chimney=True, seed=2),  # HOUSE_RED
        hip_roof(18, 22, THATCH, "thatch", seed=3),  # HOUSE_THATCH
        hip_roof(34, 28, SLATE, "tile", chimney=True, seed=4),  # TAVERN
        stall(),  # STALL
    ]
    os.makedirs(os.path.join(OUT, "ships"), exist_ok=True)
    ship_sheet(os.path.join(OUT, "ships/ships.png"))
    aura_sheet(os.path.join(OUT, "ships/aura.png"))
    os.makedirs(os.path.join(OUT, "ui"), exist_ok=True)
    frame, border = ticket_frame()
    frame.save(os.path.join(OUT, "ui/ticket.png"))
    from marvyr_icons import items_sheet

    items_sheet(os.path.join(OUT, "ui/items.png"))
    print(f"ticket: {frame.size} borda {border}px")
    rects = sheet(parts, os.path.join(OUT, "world/port-buildings.png"))
    for name, r in zip(["WAREHOUSE", "HOUSE_RED", "HOUSE_THATCH", "TAVERN", "STALL"], rects):
        print(f"{name}: rect{r}")


if __name__ == "__main__":
    main()
