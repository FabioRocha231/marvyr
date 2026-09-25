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
    img.save(dst)


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
    os.makedirs(os.path.join(OUT, "ui"), exist_ok=True)
    frame, border = ticket_frame()
    frame.save(os.path.join(OUT, "ui/ticket.png"))
    print(f"ticket: {frame.size} borda {border}px")
    rects = sheet(parts, os.path.join(OUT, "world/port-buildings.png"))
    for name, r in zip(["WAREHOUSE", "HOUSE_RED", "HOUSE_THATCH", "TAVERN", "STALL"], rects):
        print(f"{name}: rect{r}")


if __name__ == "__main__":
    main()
