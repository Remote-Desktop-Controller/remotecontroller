// RDC Maestro - Motor de Biomecânica Humana e Entropia Neuromuscular
// Gera trajetórias de mouse baseadas em curvas de Bézier cúbicas com desaceleração
// pela Lei de Fitts e atrasos estocásticos de digitação via transformada Box-Muller.

/// Gerador pseudo-aleatório simples (Xorshift64) acoplado à transformada Box-Muller
/// para produzir distribuições normais N(mean, std_dev) sem dependências externas.
pub struct BoxMullerRng {
    state: u64,
    spare: Option<f64>,
}

impl BoxMullerRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x5465_7374_5F53_6565 } else { seed },
            spare: None,
        }
    }

    /// Gera float uniforme U(0, 1) exclusivo em 0
    fn next_u01(&mut self) -> f64 {
        // Xorshift64*
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let value = self.state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let u = (value >> 11) as f64 / (1u64 << 53) as f64;
        if u <= 0.0 { 1e-7 } else if u >= 1.0 { 1.0 - 1e-7 } else { u }
    }

    /// Amostra um valor segundo a distribuição normal padrão N(0, 1)
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(val) = self.spare.take() {
            return val;
        }

        let u1 = self.next_u01();
        let u2 = self.next_u01();

        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f64::consts::PI * u2;

        let z0 = r * theta.cos();
        let z1 = r * theta.sin();

        self.spare = Some(z1);
        z0
    }

    /// Retorna um atraso em milissegundos segundo N(mean, std_dev), limitado a [min, max]
    pub fn sample_delay_ms(&mut self, mean: f64, std_dev: f64, min: f64, max: f64) -> f64 {
        let sample = mean + std_dev * self.next_gaussian();
        sample.clamp(min, max)
    }
}

/// Função de interpolação sigmoidal cúbica (Ease-In-Out) representando a Lei de Fitts:
/// aceleração suave no início, velocidade máxima no terço central e desaceleração precisa ao se aproximar do alvo.
pub fn ease_in_out(s: f64) -> f64 {
    let clamped = s.clamp(0.0, 1.0);
    if clamped < 0.5 {
        4.0 * clamped * clamped * clamped
    } else {
        1.0 - (-2.0 * clamped + 2.0).powi(3) / 2.0
    }
}

/// Calcula ponto na curva de Bézier cúbica para o parâmetro t no intervalo [0, 1]:
/// B(t) = (1-t)^3 * P0 + 3(1-t)^2 * t * P1 + 3(1-t) * t^2 * P2 + t^3 * P3
pub fn cubic_bezier(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let one_minus_t = 1.0 - t;
    let t_sq = t * t;
    let one_minus_t_sq = one_minus_t * one_minus_t;

    one_minus_t_sq * one_minus_t * p0
        + 3.0 * one_minus_t_sq * t * p1
        + 3.0 * one_minus_t * t_sq * p2
        + t_sq * t * p3
}

/// Gera uma trajetória bio-mimética contínua entre `start` e `target`.
///
/// Aplica pontos de controle com deflexão perpendicular estocástica e
/// re-amostragem temporal não-linear (Ease-In-Out) com micro-tremores.
pub fn generate_bezier_trajectory(
    start: (f64, f64),
    target: (f64, f64),
    steps: usize,
) -> Vec<(f64, f64)> {
    if steps <= 1 {
        return vec![target];
    }

    let dx = target.0 - start.0;
    let dy = target.1 - start.1;
    let distance = (dx * dx + dy * dy).sqrt();

    // Vetor perpendicular normalizado para desvio lateral da mão
    let (perp_x, perp_y) = if distance > 1e-4 {
        (-dy / distance, dx / distance)
    } else {
        (0.0, 1.0)
    };

    // Deflexão lateral proporcional à distância (20% a 35% de curvatura)
    let deflection_1 = distance * 0.25;
    let deflection_2 = -distance * 0.15;

    // Pontos de controle intermediários
    let p1 = (
        start.0 + dx * 0.33 + perp_x * deflection_1,
        start.1 + dy * 0.33 + perp_y * deflection_1,
    );
    let p2 = (
        start.0 + dx * 0.66 + perp_x * deflection_2,
        start.1 + dy * 0.66 + perp_y * deflection_2,
    );

    let mut trajectory = Vec::with_capacity(steps);
    let mut rng = BoxMullerRng::new(
        ((start.0.abs() * 1000.0) as u64)
            ^ (((target.1.abs() * 1000.0) as u64) << 16)
            ^ 0xACE1_B00B,
    );

    for i in 0..steps {
        if i == 0 {
            trajectory.push(start);
            continue;
        }
        if i == steps - 1 {
            trajectory.push(target);
            continue;
        }

        // Parâmetro de progresso linear s in (0, 1)
        let s = i as f64 / (steps - 1) as f64;
        // Tempo distorcido pela Lei de Fitts (aceleração/desaceleração)
        let t = ease_in_out(s);

        let mut x = cubic_bezier(start.0, p1.0, p2.0, target.0, t);
        let mut y = cubic_bezier(start.1, p1.1, p2.1, target.1, t);

        // Micro-tremores de alta frequência (neuromuscular tremor)
        // Amortecidos perto do início e do final para não errar o botão
        let damping = (std::f64::consts::PI * s).sin(); // 0 no início/fim, 1 no meio
        let tremor_std_dev = 1.2 * damping;
        x += rng.next_gaussian() * tremor_std_dev;
        y += rng.next_gaussian() * tremor_std_dev;

        trajectory.push((x, y));
    }

    trajectory
}
