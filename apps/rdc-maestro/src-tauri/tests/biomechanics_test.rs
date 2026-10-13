//! Testes para o motor de biomecânica e curvas de Bézier cúbicas

use rdc_maestro_lib::stealth::biomechanics::{
    BoxMullerRng, ease_in_out, generate_bezier_trajectory,
};

#[test]
fn bezier_trajectory_starts_and_ends_at_exact_coordinates() {
    let start = (100.0, 150.0);
    let target = (800.0, 600.0);
    let steps = 50;

    let points = generate_bezier_trajectory(start, target, steps);

    assert_eq!(points.len(), steps);
    let first = points.first().unwrap();
    let last = points.last().unwrap();

    // Primeiro ponto deve ser a origem
    assert!((first.0 - start.0).abs() < 1e-4);
    assert!((first.1 - start.1).abs() < 1e-4);

    // Último ponto deve ser exatamente o destino
    assert!((last.0 - target.0).abs() < 1e-4);
    assert!((last.1 - target.1).abs() < 1e-4);
}

#[test]
fn bezier_trajectory_exhibits_fitts_law_deceleration() {
    let start = (0.0, 0.0);
    let target = (1000.0, 1000.0);
    let steps = 100;

    let points = generate_bezier_trajectory(start, target, steps);

    // Calcula as distâncias percorridas a cada passo
    let step_distances: Vec<f64> = points
        .windows(2)
        .map(|w| {
            let dx = w[1].0 - w[0].0;
            let dy = w[1].1 - w[0].1;
            (dx * dx + dy * dy).sqrt()
        })
        .collect();

    // A velocidade máxima deve ocorrer no terço médio da trajetória (não no final)
    let mid_start = steps / 4;
    let mid_end = (3 * steps) / 4;
    let max_idx = step_distances
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap();

    assert!(
        max_idx >= mid_start && max_idx <= mid_end,
        "A aceleração máxima deve ocorrer no meio da trajetória (max em {} de {})",
        max_idx,
        steps
    );

    // O último passo deve ter velocidade menor que a velocidade máxima (desaceleração no alvo)
    let last_distance = *step_distances.last().unwrap();
    let max_distance = step_distances[max_idx];
    assert!(
        last_distance < max_distance,
        "Deve desacelerar ao se aproximar do alvo (último: {}, max: {})",
        last_distance,
        max_distance
    );
}

#[test]
fn ease_in_out_is_bounded_and_monotonic() {
    assert_eq!(ease_in_out(0.0), 0.0);
    assert_eq!(ease_in_out(1.0), 1.0);

    let mut prev = 0.0;
    for i in 1..=100 {
        let s = i as f64 / 100.0;
        let t = ease_in_out(s);
        assert!(
            t >= prev,
            "Curva ease_in_out deve ser monotonicamente não-decrescente"
        );
        assert!(t <= 1.0);
        prev = t;
    }
}

#[test]
fn box_muller_generates_bounded_gaussian_delays() {
    let mut rng = BoxMullerRng::new(12345);
    let mean = 85.0;
    let std_dev = 25.0;
    let min = 40.0;
    let max = 220.0;

    let mut samples = Vec::new();
    for _ in 0..500 {
        let delay = rng.sample_delay_ms(mean, std_dev, min, max);
        assert!(
            delay >= min && delay <= max,
            "Delay {} fora dos limites [{}, {}]",
            delay,
            min,
            max
        );
        samples.push(delay);
    }

    let avg: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
    // Média de 500 amostras deve estar próxima da média teórica
    assert!(
        (avg - mean).abs() < 8.0,
        "Média das amostras ({}) desviou muito da média esperada ({})",
        avg,
        mean
    );
}
