// Mar e terra do Marvyr (MF-058). Um único quad cobre o mundo; a terra
// vem das mesmas `LandMass` que o servidor usa para colisão, então o que o
// jogador vê é exatamente onde o casco encalha. Pixelado em grade de 1 m
// para casar com os sprites pixel art: paleta fechada, transições com
// dither ordenado (Bayer 4x4) e luz fixa vinda do noroeste.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput
#import bevy_sprite::mesh2d_view_bindings::globals

struct SeaParams {
    land: array<vec4<f32>, 64>,
    safe: array<vec4<f32>, 4>,
    // x: nº de discos de terra, y: nº de círculos protegidos, z: perigo 0..1
    info: vec4<f32>,
};

@group(2) @binding(0) var<uniform> sea: SeaParams;

// Luz do noroeste (mundo com y para cima).
const LIGHT: vec2<f32> = vec2<f32>(-0.7071, 0.7071);

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn hash2(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(hash(p), hash(p + vec2<f32>(19.19, 7.31)));
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * noise(q);
        q = q * 2.03 + vec2<f32>(17.0, 9.0);
        a *= 0.5;
    }
    return v;
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

// Limiar do dither ordenado 4x4, em 0..1.
fn bayer(p: vec2<f32>) -> f32 {
    let x = i32(p.x - 4.0 * floor(p.x / 4.0));
    let y = i32(p.y - 4.0 * floor(p.y / 4.0));
    var m = array<f32, 16>(
        0.0, 8.0, 2.0, 10.0,
        12.0, 4.0, 14.0, 6.0,
        3.0, 11.0, 1.0, 9.0,
        15.0, 7.0, 13.0, 5.0
    );
    return (m[y * 4 + x] + 0.5) / 16.0;
}

// Índice de paleta com dither: `v` contínuo vira degrau inteiro; só uma
// faixa estreita na fronteira entre degraus é quebrada no padrão Bayer
// (dither no plano inteiro vira textura de tela visto de longe).
fn dstep(v: f32, p: vec2<f32>) -> f32 {
    return floor(v + (bayer(p) - 0.5) * 0.35 + 0.5);
}

// x: distância com sinal até a terra (negativa dentro), y: 1 se rochedo,
// zw: normal para fora da massa mais próxima.
fn land_field(p: vec2<f32>) -> vec4<f32> {
    var d = 1e5;
    var nearest = 1e5;
    var rocky = 0.0;
    var normal = vec2<f32>(0.0, 1.0);
    let count = i32(sea.info.x);
    for (var i = 0; i < count; i++) {
        let disc = sea.land[i];
        let offset = p - disc.xy;
        let di = length(offset) - disc.z;
        d = smin(d, di, 24.0);
        if (di < nearest) {
            nearest = di;
            normal = offset / max(length(offset), 0.001);
            // Rochedo pequeno ou penhasco de instância (w = 1).
            rocky = select(0.0, 1.0, disc.z < 40.0 || disc.w > 0.5);
        }
    }
    // Costa irregular: ruído grande na terra firme, fino nos rochedos.
    let freq = select(0.018, 0.09, rocky > 0.5);
    let warp = select(16.0, 6.0, rocky > 0.5);
    d += (fbm(p * freq) - 0.5) * warp * 2.0;
    return vec4<f32>(d, rocky, normal);
}

fn water_color(level: f32) -> vec3<f32> {
    // Raso turquesa → alto-mar marinho, 5 tons.
    if (level < 0.5) { return vec3<f32>(0.44, 0.83, 0.85); }
    if (level < 1.5) { return vec3<f32>(0.25, 0.69, 0.81); }
    if (level < 2.5) { return vec3<f32>(0.16, 0.53, 0.75); }
    if (level < 3.5) { return vec3<f32>(0.12, 0.39, 0.64); }
    return vec3<f32>(0.09, 0.25, 0.48);
}

fn grass_color(level: f32) -> vec3<f32> {
    if (level < 0.5) { return vec3<f32>(0.25, 0.54, 0.24); }
    if (level < 1.5) { return vec3<f32>(0.35, 0.66, 0.28); }
    return vec3<f32>(0.49, 0.77, 0.35);
}

fn stone_color(level: f32) -> vec3<f32> {
    if (level < 0.5) { return vec3<f32>(0.24, 0.23, 0.24); }
    if (level < 1.5) { return vec3<f32>(0.37, 0.36, 0.34); }
    if (level < 2.5) { return vec3<f32>(0.55, 0.53, 0.48); }
    return vec3<f32>(0.73, 0.70, 0.64);
}

fn water(p: vec2<f32>, d: f32, n: vec2<f32>, t: f32) -> vec3<f32> {
    // Profundidade + ondulação lenta, em degraus com dither.
    let depth = clamp(d / 190.0, 0.0, 1.0);
    let swell = fbm(p * 0.010 + vec2<f32>(t * 0.015, t * 0.010)) - 0.5;
    var col = water_color(dstep((depth + swell * 0.22) * 4.0, p));

    // Sombra da ilha na água, do lado oposto à luz.
    let facing = dot(n, LIGHT);
    if (d < 9.0 && facing < -0.15) {
        col = col * 0.78;
    }

    // Brilhos esparsos: tracinhos horizontais que acendem e apagam.
    let cell = vec2<f32>(22.0, 13.0);
    let drift = vec2<f32>(t * 1.2, 0.0);
    let q = p + drift;
    let id = floor(q / cell);
    let r = hash2(id);
    let spot = floor((id + vec2<f32>(0.2 + r.x * 0.6, 0.3 + r.y * 0.4)) * cell);
    let local = floor(q) - spot;
    let len = 2.0 + floor(r.y * 4.0);
    let pulse = sin(t * 1.4 + r.x * 6.283);
    let open = select(0.0, 1.0, r.x > 0.55 && d > 14.0 && pulse > 0.25);
    if (open > 0.5 && local.y == 0.0 && abs(local.x) <= len * pulse) {
        col = mix(col, vec3<f32>(0.75, 0.93, 0.96), 0.85);
        if (local.x == 0.0 && pulse > 0.85) {
            col = vec3<f32>(0.95, 1.0, 1.0);
        }
    }

    // Ondas batendo na costa: linhas que andam para a praia e somem.
    let lap = fract((d + t * 5.0) / 15.0);
    if (d < 24.0 && lap < 0.07 && hash(floor(p * 0.5)) > d / 30.0) {
        col = mix(col, vec3<f32>(0.90, 0.98, 1.0), 0.7);
    }
    // Espuma na linha d'água, quebrada em pixels.
    let foam_w = 2.0 + 1.2 * sin(t * 1.3 + fbm(p * 0.05) * 9.0);
    if (d < foam_w) {
        col = vec3<f32>(0.93, 0.98, 1.0);
    } else if (d < foam_w + 3.0 && hash(floor(p) + floor(t * 3.0)) > 0.7) {
        col = mix(col, vec3<f32>(0.93, 0.98, 1.0), 0.6);
    }

    // Alto-mar sem lei: água mais escura e fria.
    col = mix(col, col * vec3<f32>(0.76, 0.72, 0.86), sea.info.z * smoothstep(40.0, 160.0, d));

    // Águas protegidas: anel tracejado discreto marcando o limite.
    let safe_count = i32(sea.info.y);
    for (var i = 0; i < safe_count; i++) {
        let c = sea.safe[i];
        let rr = length(p - c.xy);
        let a = atan2(p.y - c.y, p.x - c.x);
        let dash = fract(a * c.z / 18.0) < 0.5;
        if (abs(rr - c.z) < 1.2 && dash) {
            col = mix(col, vec3<f32>(0.80, 1.0, 0.86), 0.45);
        }
    }
    return col;
}

fn rock(p: vec2<f32>, d: f32, n: vec2<f32>) -> vec3<f32> {
    // Topo de pedra em estratos, borda iluminada a noroeste e face escura
    // no sudeste; musgo por cima.
    let strata = fbm(vec2<f32>(p.x * 0.05, p.y * 0.14)) * 2.2 + 0.9;
    var col = stone_color(dstep(strata, p));
    let facing = dot(n, LIGHT);
    if (d > -4.0) {
        col = select(stone_color(0.0), stone_color(3.0), facing > 0.1);
    } else if (d > -7.0 && facing < -0.1) {
        col = stone_color(1.0);
    }
    if (d < -6.0 && fbm(p * 0.09 + 5.0) > 0.58) {
        col = select(vec3<f32>(0.29, 0.45, 0.25), vec3<f32>(0.40, 0.57, 0.30), bayer(p) > 0.5);
    }
    return col;
}

fn sand(p: vec2<f32>, d: f32) -> vec3<f32> {
    // Molhada na água, seca atrás, com dither entre as duas.
    let v = clamp(-d / 9.0, 0.0, 1.0) * 2.0;
    let level = dstep(v, p);
    var col = vec3<f32>(0.79, 0.66, 0.47);
    if (level > 0.5) { col = vec3<f32>(0.89, 0.78, 0.55); }
    if (level > 1.5) { col = vec3<f32>(0.94, 0.86, 0.65); }
    if (hash(floor(p)) > 0.985) {
        col = vec3<f32>(0.74, 0.60, 0.43);
    }
    return col;
}

// Copas de árvore em células com sorteio, cada uma com luz e sombra.
// x: 1 dentro de copa, y: tom (0..3), z: 1 na sombra projetada.
fn canopy(p: vec2<f32>) -> vec3<f32> {
    let cell = 11.0;
    let base = floor(p / cell);
    var inside = 0.0;
    var tone = 0.0;
    var shadow = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let id = base + vec2<f32>(f32(x), f32(y));
            let r = hash2(id * 1.7 + 3.0);
            if (fbm(id * 0.22 + 11.0) < 0.47) {
                continue;
            }
            let c = (id + 0.2 + r * 0.6) * cell;
            let radius = 4.5 + r.x * 2.5;
            let off = p - c;
            let dist = length(off);
            if (dist < radius) {
                inside = 1.0;
                let lit = dot(off / radius, LIGHT);
                tone = 1.0;
                if (lit > 0.35) { tone = 2.0; }
                if (lit > 0.65 && dist > radius * 0.4) { tone = 3.0; }
                if (lit < -0.35 || dist > radius - 1.0) { tone = 0.0; }
            } else if (length(off + vec2<f32>(-2.0, 2.0)) < radius) {
                shadow = 1.0;
            }
        }
    }
    return vec3<f32>(inside, tone, shadow);
}

fn grass(p: vec2<f32>, d: f32, n: vec2<f32>) -> vec3<f32> {
    let v = fbm(p * 0.045) * 3.0 - 0.2;
    var col = grass_color(dstep(v, p));
    // Tufos soltos.
    if (hash(floor(p)) > 0.975) {
        col = grass_color(0.0);
    }
    // Barranco da grama sobre a areia: aceso a noroeste, escuro a sudeste.
    if (d > -14.0) {
        col = select(vec3<f32>(0.19, 0.43, 0.22), grass_color(2.0), dot(n, LIGHT) > 0.2);
    }
    // Mata no interior (mais densa longe da praia).
    if (d < -30.0) {
        let tree = canopy(p);
        if (tree.x > 0.5) {
            if (tree.y < 0.5) { col = vec3<f32>(0.08, 0.24, 0.11); }
            else if (tree.y < 1.5) { col = vec3<f32>(0.14, 0.40, 0.16); }
            else if (tree.y < 2.5) { col = vec3<f32>(0.24, 0.55, 0.20); }
            else { col = vec3<f32>(0.43, 0.70, 0.26); }
        } else if (tree.z > 0.5) {
            col = col * vec3<f32>(0.62, 0.70, 0.62);
        }
    }
    return col;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let p = floor(mesh.world_position.xy) + 0.5;
    let t = globals.time;
    let field = land_field(p);
    let d = field.x;
    let n = field.zw;

    var col: vec3<f32>;
    if (d > 0.0) {
        col = water(p, d, n, t);
    } else if (field.y > 0.5) {
        col = rock(p, d, n);
    } else if (d > -10.0) {
        col = sand(p, d);
    } else {
        col = grass(p, d, n);
    }
    return vec4<f32>(col, 1.0);
}
